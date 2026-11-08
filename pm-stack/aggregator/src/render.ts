// Pure HTML rendering. Every view function takes already-assembled data and
// returns a full HTML string — nothing here does I/O, which keeps the smoke
// tests cheap (render from fixtures and assert on output).
import {
  type AttentionView,
  type RoadmapGrain,
  type TodayView,
  allRefs,
  commentStream,
  executionStrip,
  freshness,
  monthTimeline,
  projectPace,
  resetMarkers,
  roadmapDayGrain,
  sessionChildren,
  sessionRoots,
  sourceOf,
  STATE_LABELS,
  timeline as buildTimeline,
  windowPercent,
} from "./derive.ts";
import type { RoadmapReconciliation } from "./roadmap-reconcile.ts";
import type { RoadmapAnnotation } from "./roadmap-annotations.ts";
import { diffCoverage, type CoverageSnapshotFile } from "./coverage.ts";
import { DIGEST_SECTION_LIMIT, type Digest, type DigestKind } from "./digest.ts";
import { registryQueryString, type RegistryView } from "./registry-view.ts";
import { latestInFamily, newClientToken, type DashAction } from "./actions.ts";
import { applyBoardDonePolicy, DONE_STRIP_RENDER_LIMIT } from "./board-policy.ts";
import { cardState, pendingVerb, type CardState } from "./card-state.ts";
import {
  DELEGATION_DEFAULT_OBJECT_REF,
  NEEDS_ORDER_OBJECT_REF,
  effectiveStandingAuto,
  proposalExpired,
  proposalIdentity,
  questionIdForOption,
  type NeedsOrder,
} from "./tier-items.ts";
import { commentKindCounts, githubCommentUrl, queuedComments, type CommentKindCounts } from "./comments.ts";
import { serviceHomeLinks, type ItemServiceLink } from "./service-links.ts";
import { activityCoverageHtml, gapsHtml, type ActivityCoverage, type GapRecord } from "./activity.ts";
import type { AuditEvent } from "./events.ts";
import { aaCaiPoints, aaPoints, summarizeAa } from "./aa.ts";
import {
  applyInequalityFilters,
  bandIndex,
  computeRecommendations,
  logEdges,
  normName,
  paretoSet,
  pctRank,
  projectTask,
  scoreEdges,
} from "./assets/aa-explorer-computations.js";
import { summarizeDns } from "./dns.ts";
import type { NeedsDrewCensus } from "./needs-drew.ts";
import type { Incident, IncidentEvent, PushSubscriptionRow } from "./alerts.ts";
import type { NotificationRollup } from "./notifications.ts";
import { ageMsText, feedAgeText } from "./feed.ts";
import { providerDisplayNames } from "./provider-display.ts";
import type { QueueView, QueueRow } from "./queue-view.ts";
import type { HealthView, PageHealth } from "./health.ts";
import {
  type AaCaiRow,
  type AaConfig,
  type AaFeedSection,
  type AaRanking,
  type AaSection,
  type AaTestRun,
  type BoardItem,
  type ChargeRow,
  type Comment,
  type DashAuthor,
  type DashComment,
  type DelegationMode,
  type DnsEntry,
  type DnsSection,
  type GhItem,
  type OrchestrationSection,
  type PaygLine,
  type ProjectPoolItem,
  type Provenance,
  type ProvenanceDetail,
  type QuotaState,
  type QuotaView,
  type RefKey,
  type RoadmapRow,
  type SaasService,
  type SessionRecord,
  type SnapshotRecord,
  type SourceCoverage,
  type TieredEntry,
  type TierItem,
  type TierItemQuestion,
  type UsageMetric,
  type UsageView,
  type WhyUserClass,
  SAAS_GROUP_LABELS,
  SESSION_SOURCE_LABELS,
  countedEstimate,
  refsInText,
  isValidRef,
  refUrl,
} from "./model.ts";
import { PAGES, groupedPages, primaryGroupedPages, manifestRouteFor, pageByRoute } from "./pages.ts";
import { KEY_HINT, KEYBOARD_SCRIPT, paletteHtml } from "./keyboard.ts";
import type { ProgramData, ProgramEntry, ProgramLive, TrackedIssue } from "./program.ts";
import type { ProjectDetail, ProjectSummary } from "./project-config.ts";
import { derivedProjectState, projectDisagreement, PROJECT_URL } from "./projects.ts";
import type { ConflictEntry, ConflictInbox } from "./conflicts.ts";
import { buildItemBrief, type ItemBrief } from "./brief.ts";
import { buildEpicRollups, isEpicChildDone, type EpicRollup } from "./epics.ts";
import { effortWeight, handsLine, summarizeWeights, weightedLine } from "./weights.ts";
import { blockedSummary, depthLayout, hasGraphEdges } from "./graph.ts";
import { EPIC_GRAPH_DEFAULT_CAP, epicGraph, epicSelector } from "./epic-graph.ts";
import { type AuthorIdentity } from "./registry.ts";
import type { SearchResult } from "./search-view.ts";
import { dnsAnchor } from "./search-view.ts";

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
const DASH_AFTER_JS = `
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
const COMMENT_SCRIPT = `<script>${DASH_AFTER_JS}
document.addEventListener("submit", async (ev) => {
  const form = ev.target;
  if (!(form instanceof HTMLFormElement) || !form.classList.contains("cform")) return;
  ev.preventDefault();
  const err = form.querySelector(".err");
  const data = new FormData(form);
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
    const res = await fetch("/api/comments", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(p) });
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
document.addEventListener("change", (ev) => {
  const t = ev.target;
  if (!(t instanceof HTMLInputElement) || t.name !== "kind") return;
  const form = t.closest(".cform");
  const wrap = form && form.querySelector(".ghtarget");
  if (wrap) wrap.style.display = t.value === "github" && t.checked ? "" : "none";
});
</script>`;

const ROADMAP_ANNOTATION_SCRIPT = `<script>${DASH_AFTER_JS}
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
const ACTION_SCRIPT = `<script>${DASH_AFTER_JS}
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

function layout(
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
</main><script src="/assets/push-client.js?v=push-ux-2026-11-01" defer></script>${extra}${paletteHtml()}${KEYBOARD_SCRIPT}</body></html>`;
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
    if (result.kind === "service") { chipKeys.add("quota"); chipKeys.add("usage"); }
    if (result.kind === "tier") { chipKeys.add("orchestration"); chipKeys.add("attention"); chipKeys.add("registry"); }
  }
  return layout(
    "Search",
    body,
    provenance,
    "/search",
    "",
    detail ? freshnessChips(detail, [...chipKeys]) : "",
  );
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

export function authorBadge(a: DashAuthor): string {
  const cls = a.kind === "drew" ? "drew" : a.kind === "muse" ? "Muse" : "agent";
  const link = a.sessionId
    ? ` <a href="/sessions?id=${encodeURIComponent(a.sessionId)}#${encodeURIComponent(a.sessionId)}" class="muted">session</a>`
    : "";
  return `<span class="badge ${cls}">${esc(a.label)}</span>${link}`;
}

export function dashCommentHtml(c: DashComment, showObject = false): string {
  const kindChip =
    c.kind === "message"
      ? `<span class="badge msg">✉ message to Muse</span> <span class="muted">${c.triageState === "triaged" ? "triaged by the orchestrator" : "awaiting triage"}</span>`
      : c.kind === "github"
        ? `<span class="badge gh">GitHub comment</span> <span class="muted">${
            c.githubState === "posted"
              ? `posted${c.githubUrl ? ` — <a href="${esc(c.githubUrl)}">view</a>` : ""}`
              : c.githubState === "failed"
                ? `posting failed ${relTime(c.createdAt)} — no automatic retry exists; a fleet poster must post it by hand${c.githubError ? ` — last error: ${esc(c.githubError)}` : ""}`
                : `pending — will post to ${esc(c.githubRef ?? "")}`
          }</span>`
        : `<span class="badge">note</span>`;
  const obj = showObject ? ` <span class="muted">on <code>${esc(c.objectRef)}</code></span>` : "";
  return `<div class="cmt ${esc(c.kind)}">${authorBadge(c.author)} ${kindChip}${obj}<br>${esc(c.body)}<br><span class="muted">${fmtDate(c.createdAt)}</span></div>`;
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

function authorSelectHtml(identities: AuthorIdentity[]): string {
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

function aformHtml(spec: AFormSpec): string {
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
function actboxHtml(opts: ActionFormOpts, forms: string): string {
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

/** The Project Status values a status request may target (#910 field). */
export const PROJECT_STATUS_VALUES = ["Todo", "In Progress", "In Review", "Done"] as const;

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

function itemBadge(item: GhItem): string {
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

function stripHtml(items: Map<RefKey, GhItem>, links: BoardItem["links"]): string {
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

function briefPanelsForRefs(refs: RefKey[], briefs?: Map<RefKey, ItemBrief>): string {
  return [...new Set(refs)]
    .map((ref) => briefs?.get(ref))
    .filter((brief): brief is ItemBrief => Boolean(brief))
    .map(briefPanelHtml)
    .join("");
}

// --- Board view -----------------------------------------------------------

/** The board's card grid alone (#1444 fragment payload). */
export function renderBoardBody(
  board: BoardItem[],
  items: Map<RefKey, GhItem>,
  counts?: Map<string, number | CommentKindCounts>,
  act?: {
    actionsByObject?: Map<string, DashAction[]>;
    poolByRef?: Map<RefKey, ProjectPoolItem>;
    identities?: AuthorIdentity[];
    briefs?: Map<RefKey, ItemBrief>;
  },
  renderOpts?: { showAllDone?: boolean },
): string {
  // #1146: finished work is not WIP — aged-out done items and the
  // retired ROADMAP Done-table class never reach the board, whatever
  // source assembled it (the Kaneo source filters too; see
  // board-policy.ts). Surviving done renders collapsed, below.
  board = applyBoardDonePolicy(board);
  const cols = new Map<string, BoardItem[]>();
  for (const item of board) {
    const key = item.status === "done" ? "done" : item.status;
    const k = key === "to-do" ? "to-do" : key;
    cols.set(k, [...(cols.get(k) ?? []), item]);
  }
  const order = ["in-flight", "to-do", "in-progress", "in-review", "done"];
  const extra = [...cols.keys()].filter((k) => !order.includes(k));
  const bodies = [...order, ...extra]
    .filter((k) => cols.has(k))
    .map((k) => {
      const colItems = cols.get(k) ?? [];
      // #1179: the done strip renders only the most recent
      // DONE_STRIP_RENDER_LIMIT cards (newest first); the rest are
      // one click away (?done=all). Rendering all ~950 cost ~1 MB.
      let visibleItems = colItems;
      let doneNote = "";
      if (k === "done" && colItems.length > DONE_STRIP_RENDER_LIMIT) {
        const sorted = [...colItems].sort(
          (a, b) => (Date.parse(b.updatedAt ?? "") || 0) - (Date.parse(a.updatedAt ?? "") || 0),
        );
        if (renderOpts?.showAllDone) {
          visibleItems = sorted;
          doneNote = ` — showing all, newest first; <a href="/board">show fewer</a>`;
        } else {
          visibleItems = sorted.slice(0, DONE_STRIP_RENDER_LIMIT);
          doneNote = ` — showing the ${DONE_STRIP_RENDER_LIMIT} most recent; <a href="/board?done=all">show all ${colItems.length}</a>`;
        }
      }
      const cards = visibleItems
        .map((item) => {
          const repo = item.repo ? `<span class="muted">${esc(item.repo)}</span><br>` : "";
          const cmt = counts ? commentBadge(item.id, counts) : "";
          const briefs = briefPanelsForRefs(
            item.links.map((link) => link.ref),
            act?.briefs,
          );
          if (item.source === "orchestrator") {
            // Synthetic in-flight entry: visibly not a Kaneo card. Its
            // GitHub refs link directly (no execution strip — the
            // projector may not know these refs yet).
            const status = item.orchStatus ? `<span class="badge src">${esc(item.orchStatus)}</span> ` : "";
            const goal = item.goal ? `<br><span class="muted">goal: ${esc(item.goal)}</span>` : "";
            const blocked = item.blockedBy?.length
              ? `<br><strong>Blocked by:</strong> ${item.blockedBy.map(esc).join("; ")}`
              : "";
            const refs = item.links.length
              ? `<div class="strip">${item.links.map((l) => `<a class="badge open" href="${esc(refUrl(l.ref))}">${esc(l.ref)}</a>`).join(" ")}</div>`
              : "";
            return `<div class="card"><a href="/item/${encodeURIComponent(item.id)}"><span class="badge implementing">orchestrator · in flight</span> ${esc(item.title)}</a>${cmt}<br>${status}${repo}${goal}${blocked}${refs}${briefs}</div>`;
          }
          const src = item.source === "github" ? `<span class="badge open">gh-only</span> ` : "";
          // #1154: a card whose linked issue sits in the Project pool
          // carries the status-request control for that pool item.
          let statusCtl = "";
          if (act?.identities && act.poolByRef) {
            const pooled = item.links.map((l) => act.poolByRef!.get(l.ref)).find((p) => p !== undefined);
            if (pooled) {
              const objectRef = ghObjectRef(pooled.ref);
              statusCtl = `${actionBadgesHtml(act.actionsByObject?.get(objectRef))}${statusRequestHtml(objectRef, pooled.ref, pooled.status ?? null, { identities: act.identities })}`;
            }
          }
          const kaneo = item.kaneoUrl ? ` <a class="badge" href="${esc(item.kaneoUrl)}">Kaneo ↗</a>` : "";
          return `<div class="card"><a href="/item/${encodeURIComponent(item.id)}">${src}${esc(item.title)}</a>${cmt}<br>${repo}${kaneo}${stripHtml(items, item.links)}${statusCtl}${briefs}</div>`;
        })
        .join("");
      // The in-flight lane is dated by the orchestration section that
      // produced it (stamped on every synthetic entry as updatedAt).
      if (k === "done") {
        // Collapsed by default (#1146): a count + the last 7 days
        // behind an expander, never a full column of finished cards.
        return `<details class="donestrip"><summary><strong>Done recently (${colItems.length})</strong> — done in the last 7 days, collapsed by default; older done is aged off the board (GitHub is the record)${doneNote}</summary>${cards}</details>`;
      }
      const heading =
        k === "in-flight"
          ? `in flight — orchestrator · in flight · section ${fmtDate(colItems[0]?.updatedAt)} (${colItems.length})`
          : `${esc(k)} (${colItems.length})`;
      return `<div class="col"><h2>${heading}</h2>${cards}</div>`;
    })
    .join("");
  return `<div class="board">${bodies}</div>`;
}

/** The board page (#1444): full server render. */
export function renderBoard(
  board: BoardItem[],
  items: Map<RefKey, GhItem>,
  provenance: Provenance,
  counts?: Map<string, number | CommentKindCounts>,
  detail?: ProvenanceDetail,
  act?: {
    actionsByObject?: Map<string, DashAction[]>;
    poolByRef?: Map<RefKey, ProjectPoolItem>;
    identities?: AuthorIdentity[];
    briefs?: Map<RefKey, ItemBrief>;
  },
  renderOpts?: { showAllDone?: boolean },
): string {
  return layout(
    "Cross-repo WIP board",
    `<h1>Cross-repo WIP board</h1><p class="muted">What is being worked on now is the <strong>in-flight</strong> lane: synthetic entries from the orchestrator's lined-up tasks (the fleet feed's orchestration section), not Kaneo cards. The other columns are the curated Kaneo board across the four repos, including its completion history — on their own they are a record of finished and in-review work, not a live WIP list. Each Kaneo card's execution strip is derived from its linked issues/PRs: implementing → review → merged.</p>${renderBoardBody(board, items, counts, act, renderOpts)}`,
    provenance,
    "/board",
    act?.identities ? ACTION_SCRIPT : "",
    detail ? freshnessChips(detail, ["board", "items", "orchestration"]) : "",
  );
}

/** The Datastar shell for / (#1444): heading + intro paint at once;
 *  the board itself streams in from /frag/board. */
export function renderBoardShell(provenance: Provenance, detail?: ProvenanceDetail): string {
  return layout(
    "Cross-repo WIP board",
    `<h1>Cross-repo WIP board</h1><p class="muted">What is being worked on now is the <strong>in-flight</strong> lane: synthetic entries from the orchestrator's lined-up tasks (the fleet feed's orchestration section), not Kaneo cards.</p><noscript><meta http-equiv="refresh" content="0; url=/board"></noscript><div id="board-main" data-frag data-on-load="@get('/frag/board')" data-on-dashrefresh__window="@get('/frag/board')"><div class="skel">Loading the board…</div></div>`,
    provenance,
    "/board",
    ACTION_SCRIPT,
    detail ? freshnessChips(detail, ["board", "items", "orchestration"]) : "",
  );
}

// --- Roadmap view ----------------------------------------------------------

const GRAINS: [RoadmapGrain, string][] = [
  ["quarter", "Quarter"],
  ["month", "Month"],
  ["week", "Week"],
  ["3d", "Next 3 days"],
  ["today", "Today"],
];

/** One roadmap row's drill-down (#1168): its matched Project epic
 *  (absent when the row is unmatched) and the epic's pool children —
 *  the pool items whose epicRef names the epic (#2399). */
export interface RoadmapDrill {
  epic?: ProjectPoolItem;
  children: ProjectPoolItem[];
}

/** Build the per-row drill-down index from a reconciliation and the
 *  Project pool, keyed by the row's registry subject. Every row gets
 *  an entry, so an unmatched row is distinguishable from drill data
 *  never having been supplied. Callers pass no index at all when the
 *  reconciliation is unavailable — a failed source read must not
 *  masquerade as "no epic yet" (#1144's availability rule). */
export function buildRoadmapDrill(
  rows: RoadmapRow[],
  reconciliation: RoadmapReconciliation,
  pool: ProjectPoolItem[],
): Map<string, RoadmapDrill> {
  const epicByRow = new Map(reconciliation.matched.map((m) => [m.row.subject, m.epic] as const));
  const childrenByEpic = new Map<string, ProjectPoolItem[]>();
  for (const item of pool) {
    if (!item.epicRef) continue;
    const list = childrenByEpic.get(item.epicRef) ?? [];
    list.push(item);
    childrenByEpic.set(item.epicRef, list);
  }
  return new Map(
    rows.map((row) => {
      const epic = epicByRow.get(row.subject);
      return [row.subject, { epic, children: epic ? (childrenByEpic.get(epic.ref) ?? []) : [] }] as const;
    }),
  );
}

function roadmapDrilldown(drill: RoadmapDrill, row: RoadmapRow): string {
  const parts: string[] = [];
  if (!drill.epic) {
    parts.push(`<p class="muted">No epic yet — no Project epic matches this roadmap row.</p>`);
  } else {
    const epic = drill.epic;
    parts.push(
      `<p>Epic: <a href="${esc(epic.issueUrl)}">${esc(epic.title)}</a> <span class="muted">${esc(epic.ref)} · ${esc(epic.state)}</span></p>`,
    );
    if (drill.children.length === 0) {
      parts.push(`<p class="muted">No child issues in the Project pool for this epic yet.</p>`);
    } else {
      const done = drill.children.filter(isEpicChildDone).length;
      const total = drill.children.length;
      const pct = Math.round((done / total) * 100);
      parts.push(
        `<p class="muted">Progress: ${done}/${total} child issues done</p><div class="progress"><progress value="${done}" max="${total}">${pct}%</progress></div>`,
      );
      parts.push(
        `<ul>${drill.children
          .map(
            (c) =>
              `<li><a href="${esc(c.issueUrl)}">${esc(c.title)}</a> <span class="badge ${esc(c.state)}">${esc(c.state)}</span> <span class="muted">${esc(c.ref)}</span></li>`,
          )
          .join("")}</ul>`,
      );
    }
  }
  if (row.notes.trim()) parts.push(`<p class="notes">${esc(row.notes)}</p>`);
  return `<details class="roadmap-drill"><summary>Epic &amp; issues</summary>${parts.join("")}</details>`;
}

function roadmapCard(
  row: RoadmapRow,
  items: Map<RefKey, GhItem>,
  counts?: Map<string, number | CommentKindCounts>,
  drill?: RoadmapDrill,
  annotations?: Map<string, RoadmapAnnotation[]>,
  briefs?: Map<RefKey, ItemBrief>,
): string {
  const refs = allRefs([row]);
  const refBadges = refs
    .map((r) => items.get(r))
    .filter((x): x is GhItem => Boolean(x))
    .map(itemBadge)
    .join(" ");
  const cmt = counts ? commentBadge(row.subject, counts) : "";
  const grist = row.gristUrl ? ` <a class="badge" href="${esc(row.gristUrl)}">Grist ↗</a>` : "";
  const expansion = drill ? roadmapDrilldown(drill, row) : "";
  const notes = annotations?.get(row.subject) ?? [];
  const annotationHtml = notes.length
    ? `<div class="roadmap-annotations">${notes.map(roadmapAnnotationHtml).join("")}</div>`
    : "";
  const briefPanels = briefPanelsForRefs(refs, briefs);
  return `<div class="card"><a href="/item/${encodeURIComponent(row.subject)}">${esc(row.title)}</a>${cmt}<br><span class="badge ${esc(row.status.toLowerCase().replace(/[^a-z]/g, "-"))}">${esc(row.status)}</span> <span class="muted">${esc(row.area)} · ${esc(row.horizon)}</span>${refBadges}${grist}${annotationHtml}${expansion}${briefPanels}</div>`;
}

function roadmapAnnotationHtml(annotation: RoadmapAnnotation): string {
  return `<div class="roadmap-annotation"><span class="muted">${esc(annotation.date)}</span> <span class="badge">${esc(annotation.kind)}</span> ${authorBadge(annotation.author)}<br>${esc(annotation.body)}</div>`;
}

function timelineTable(
  tl: ReturnType<typeof buildTimeline>,
  items: Map<RefKey, GhItem>,
  counts?: Map<string, number | CommentKindCounts>,
  drill?: Map<string, RoadmapDrill>,
  annotations?: Map<string, RoadmapAnnotation[]>,
  briefs?: Map<RefKey, ItemBrief>,
): string {
  const head = `<tr><th scope="col">Theme</th>${tl.lanes.map((l) => `<th scope="col">${esc(l.label)}</th>`).join("")}</tr>`;
  const body = tl.themes
    .map((theme) => {
      const cells = tl.lanes
        .map((lane) => {
          const rs = tl.cells.get(theme)?.get(lane.key) ?? [];
          return `<td class="tl-cell">${rs.map((r) => roadmapCard(r, items, counts, drill?.get(r.subject), annotations, briefs)).join("")}</td>`;
        })
        .join("");
      return `<tr><th scope="row">${esc(theme)}</th>${cells}</tr>`;
    })
    .join("");
  return `<div class="tblwrap"><table class="table table-zebra"><caption>Roadmap timeline by theme</caption>${head}${body}</table></div>`;
}

export function renderRoadmap(
  rows: RoadmapRow[],
  items: Map<RefKey, GhItem>,
  provenance: Provenance,
  grain: RoadmapGrain = "quarter",
  counts?: Map<string, number | CommentKindCounts>,
  detail?: ProvenanceDetail,
  reconciliation?: RoadmapReconciliation,
  drill?: Map<string, RoadmapDrill>,
  annotations?: Map<string, RoadmapAnnotation[]>,
  viewQuery = "",
  briefs?: Map<RefKey, ItemBrief>,
): string {
  // #1144: when the Grist rows and the Project's Kind=Epic pool items
  // disagree, say so on the page itself — drift the reader can see,
  // with the full lists published in /api/roadmap.json.
  const driftFlag =
    reconciliation?.availability === "unavailable"
      ? `<div class="flag">Roadmap reconciliation unavailable: ${esc(reconciliation.unavailableReason ?? "a roadmap source could not be read")} — no drift is asserted when a source read failed (#1144).</div>`
      : reconciliation && reconciliation.drift > 0
        ? `<div class="flag">Roadmap drift: ${reconciliation.unmatchedRows.length} Grist row${reconciliation.unmatchedRows.length === 1 ? "" : "s"} with no matching Project epic · ${reconciliation.unmatchedEpics.length} Project epic${reconciliation.unmatchedEpics.length === 1 ? "" : "s"} with no roadmap row (#1144 — the two roadmap sources disagree; both lists are in <a href="/api/roadmap.json">/api/roadmap.json</a>).</div>`
        : "";
  const chips = GRAINS.map(
    ([g, label]) =>
      `<a href="/roadmap?h=${g}${viewQuery ? `&${viewQuery}` : ""}"${g === grain ? ' class="active"' : ""}>${label}</a>`,
  ).join("");
  let body: string;
  if (grain === "quarter") {
    body = timelineTable(buildTimeline(rows), items, counts, drill, annotations, briefs);
  } else if (grain === "month") {
    body = timelineTable(monthTimeline(rows), items, counts, drill, annotations, briefs);
  } else {
    const dg = roadmapDayGrain(rows, grain);
    const sections = dg.lanes
      .map(
        (lane) =>
          `<h2>${esc(lane.label)}</h2>${lane.rows.length ? lane.rows.map((r) => roadmapCard(r, items, counts, drill?.get(r.subject), annotations, briefs)).join("") : '<p class="muted">Nothing dated this day.</p>'}`,
      )
      .join("");
    const notes: string[] = [];
    if (dg.outside > 0) notes.push(`${dg.outside} dated row${dg.outside === 1 ? "" : "s"} fall outside this window`);
    if (dg.coarse.length > 0) {
      notes.push(
        `${dg.coarse.length} row${dg.coarse.length === 1 ? "" : "s"} have no calendar date (quarter grain, ongoing, or unscheduled) — see the Quarter view`,
      );
    }
    body = `${sections}${notes.length ? `<p class="muted">${notes.join(" · ")}</p>` : ""}`;
  }
  return layout(
    "Roadmap",
    `<h1>Roadmap</h1><p class="muted">Grist's <em>Program roadmap</em> table at every grain — one data model, switchable horizon. Rows with a calendar date land on their day; quarter-labelled rows stay at quarter grain.</p><div class="chips">${chips}</div>${driftFlag}${body}`,
    provenance,
    "/roadmap",
    "",
    detail ? freshnessChips(detail, ["roadmap", "items"]) : "",
  );
}

// --- Item drill-down -------------------------------------------------------

export interface ItemExtras {
  /** Dashboard comments (#906 §2) across this item's object refs. */
  dashComments: DashComment[];
  /** Present only on roadmap rows; undefined on ordinary board items. */
  roadmapAnnotations?: RoadmapAnnotation[];
  /** The object ref the inline form posts to (the item's subject). */
  formRef: string;
  formOpts: CommentFormOpts;
  /** Registry sessions loosely linked to this item's refs (#906 §3). */
  sessions: SessionRecord[];
  /** Dashboard actions across this item's object refs (#1152/#1154). */
  actions?: DashAction[];
  /** Set when one of the item's refs is in the Project pool (#1154). */
  statusRequest?: { objectRef: string; targetRef: RefKey; currentStatus: string | null };
  /** Outbound links to the live services this item lives in (#1136). */
  serviceLinks?: ItemServiceLink[];
  /** Present when this item's Project-pool entry is an epic. */
  epicRollup?: EpicRollup;
  /** Project-pool briefs for linked refs (#1279). */
  briefs?: Map<RefKey, ItemBrief>;
  /** Real ready-for-agent dispatch state for explicitly labelled linked issues. */
  dispatchStates?: { ref: RefKey; state: string; provenance: { source: string; observedAt: string } }[];
  /** The shared audit stream, scoped to the item's canonical refs. */
  activity?: { events: AuditEvent[]; globalHref: string; gaps: GapRecord[]; coverage?: readonly ActivityCoverage[] };
}

function roadmapAnnotationFormHtml(objectRef: string, identities: AuthorIdentity[]): string {
  return `<form class="raform" data-object="${esc(objectRef)}">
<label>Annotation kind</label><select name="kind"><option value="decision">Decision</option><option value="outcome">Outcome</option><option value="context" selected>Context</option></select>
<label>Date</label><input type="date" name="date" required>
<label>Author</label><select name="author" data-author-picker>${authorOptionsHtml(identities)}</select>
<label>Annotation</label><textarea name="body" rows="3" required placeholder="Why did this change, slip, or get dropped?"></textarea>
<button type="submit">Add annotation</button><div class="err"></div></form>`;
}

function epicRollupHtml(rollup: EpicRollup): string {
  const statusMix = rollup.statusMix.length
    ? rollup.statusMix.map(({ status, count }) => `${esc(status)} ${count}`).join(" · ")
    : "none";
  const children = rollup.total
    ? `Children: <strong>${rollup.done}/${rollup.total} done</strong>`
    : `Children: <strong>0 resolved</strong> · completion unknown`;
  return `<p class="muted epic-rollup"><strong>Epic rollup</strong> — ${children} · Status mix: ${statusMix} · Blocked: ${rollup.blocked}</p>`;
}

export function eventTableHtml(events: readonly AuditEvent[]): string {
  const rows = events
    .map((event) => {
      const source = event.source.link
        ? `<a href="${esc(event.source.link)}" rel="noopener">${esc(event.source.identity)}</a>`
        : esc(event.source.identity);
      const delay = Date.parse(event.ingestedAt) - Date.parse(event.at);
      const ingested =
        Number.isFinite(delay) && delay > 0 ? `<span class="muted">(+${Math.round(delay / 1000)}s ingest)</span>` : "";
      return `<tr><td>${esc(event.at)} ${ingested}</td><td>${esc(event.actor)} <span class="muted">(${esc(event.attribution.confidence)})</span></td><td>${esc(event.kind)}</td><td><code>${esc(event.ref)}</code></td><td>${esc(event.summary)}</td><td>${source}</td></tr>`;
    })
    .join("\n");
  return `<div class="tblwrap"><table class="activity-events"><caption>Shared activity events</caption><thead><tr><th scope="col">When</th><th scope="col">Actor</th><th scope="col">Kind</th><th scope="col">Ref</th><th scope="col">Summary</th><th scope="col">Source</th></tr></thead><tbody>${rows || '<tr><td colspan="6" class="muted">No matching events; this does not establish complete coverage.</td></tr>'}</tbody></table></div>`;
}

export function renderItem(
  title: string,
  subject: string,
  links: BoardItem["links"],
  items: Map<RefKey, GhItem>,
  comments: Comment[],
  provenance: Provenance,
  extras?: ItemExtras,
): string {
  const strip = executionStrip(links, items);
  const all = [...strip.implementing, ...strip.review, ...strip.merged, ...strip.queued, ...strip.closed];
  const refRows = all
    .map(
      (i) =>
        `<tr><td>${itemBadge(i)}</td><td>${esc(i.title)}</td><td>${esc(i.labels.join(", "))}</td><td>${fmtDate(i.updatedAt)}</td></tr>`,
    )
    .join("");
  const stream = commentStream(links, comments);
  const commentRows = stream
    .map(
      (c) =>
        `<tr><td class="muted">${fmtDate(c.createdAt)}</td><td><span class="badge ${esc(c.kind)}">${esc(c.kind)}</span></td><td>${esc(c.author)}</td><td>${esc(c.body)}${c.path ? ` <span class="muted">(${esc(c.path)}${c.line ? `:${c.line}` : ""})</span>` : ""}</td></tr>`,
    )
    .join("");
  const dashSection = extras
    ? `<h2>Dashboard comments</h2>${
        extras.dashComments.length
          ? extras.dashComments.map((c) => dashCommentHtml(c, true)).join("")
          : `<p class="muted">No dashboard comments yet — notes are read when this is picked up; messages are triaged by the orchestrator.</p>`
      }${commentFormHtml(extras.formRef, extras.formOpts)}`
    : "";
  const roadmapAnnotationSection =
    extras?.roadmapAnnotations !== undefined
      ? `<section class="roadmap-annotation-section"><h2>Roadmap annotations</h2>${extras.roadmapAnnotations.length ? extras.roadmapAnnotations.map(roadmapAnnotationHtml).join("") : '<p class="muted">No roadmap annotations yet.</p>'}${roadmapAnnotationFormHtml(extras.formRef, extras.formOpts.identities)}</section>`
      : "";
  const sessionSection =
    extras && extras.sessions.length
      ? `<h2>Sessions that worked this</h2><div class="item-session-records">${extras.sessions.map((r) => sessionRecordLine(r)).join("")}</div>`
      : "";
  const statusSection =
    extras?.statusRequest && extras.formOpts
      ? `<h2>Request status change</h2>${actionBadgesHtml(extras.actions)}${statusRequestHtml(extras.statusRequest.objectRef, extras.statusRequest.targetRef, extras.statusRequest.currentStatus, { identities: extras.formOpts.identities })}`
      : "";
  const svcLinks = extras?.serviceLinks?.length
    ? `<p>Live services: ${extras.serviceLinks.map((s) => `<a href="${esc(s.url)}">${esc(s.label)} ↗</a>`).join(" · ")}</p>`
    : "";
  const epicRollup = extras?.epicRollup ? epicRollupHtml(extras.epicRollup) : "";
  const dispatchSection = extras?.dispatchStates?.length
    ? `<section><h2>ready-for-agent dispatch</h2><ul>${extras.dispatchStates
        .map(
          (entry) =>
            `<li><a href="https://github.com/${esc(entry.ref.replace("#", "/issues/"))}">${esc(entry.ref)}</a>: <strong>${esc(entry.state)}</strong> <span class="muted">· ${esc(entry.provenance.source)} checked ${esc(entry.provenance.observedAt)}</span></li>`,
        )
        .join("")}</ul></section>`
    : "";
  const activitySection = extras?.activity
    ? `<h2>Activity stream</h2><p><a href="${esc(extras.activity.globalHref)}">View these events in the global river</a></p><p class="muted"><strong>Freshness: unknown.</strong> The ingestion checkpoint records deliveries, not successful source polls. An empty event list does not establish complete coverage. Known gaps and documented source coverage are shown below.</p>${eventTableHtml(extras.activity.events)}<h3>Known activity gaps</h3><p class="muted">Gap records are source-scoped and do not currently identify affected item refs, so this is the global known-gap list.</p><div class="tblwrap">${gapsHtml(extras.activity.gaps)}</div><h3>Activity source coverage</h3><div class="tblwrap">${activityCoverageHtml(extras.activity.coverage)}</div>`
    : "";
  return layout(
    title,
    `<h1>${esc(title)}</h1><p class="muted">subject: <code>${esc(subject)}</code></p>${epicRollup}${svcLinks}${dispatchSection}${stripHtml(items, links)}
${
  extras?.briefs
    ? briefPanelsForRefs(
        links.map((link) => link.ref),
        extras.briefs,
      )
    : ""
}
<h2>Linked issues / PRs</h2><div class="tblwrap"><table class="table table-zebra"><caption>Linked issues and pull requests</caption><tr><th scope="col">Ref</th><th scope="col">Title</th><th scope="col">Labels</th><th scope="col">Updated</th></tr>${refRows || '<tr><td colspan="4" class="muted">none resolved</td></tr>'}</table></div>
 <h2>Activity stream comments</h2><div class="tblwrap"><table class="table table-zebra"><caption>Comments</caption><tr><th scope="col">When</th><th scope="col">Kind</th><th scope="col">Author</th><th scope="col">Body</th></tr>${commentRows || '<tr><td colspan="4" class="muted">no comments</td></tr>'}</table></div>
 ${activitySection}${sessionSection}${statusSection}${roadmapAnnotationSection}${dashSection}`,
    provenance,
    "/board",
    extras
      ? COMMENT_SCRIPT +
          (extras.roadmapAnnotations !== undefined ? ROADMAP_ANNOTATION_SCRIPT : "") +
          (extras.statusRequest ? ACTION_SCRIPT : "")
      : "",
  );
}

/** One registry record as a compact line (item page, sessions page). */
export function sessionRecordLine(
  r: SessionRecord,
  act?: { actions?: DashAction[]; identities?: AuthorIdentity[] },
): string {
  const tokens = r.tokensTotal != null ? ` · ${r.tokensTotal.toLocaleString()} tok` : "";
  const parent = r.parentId
    ? ` · sub of <a href="/sessions?id=${encodeURIComponent(r.parentId)}#${encodeURIComponent(r.parentId)}">${esc(r.parentId)}</a>`
    : "";
  const renderedRefs = r.taskRefs.flatMap((ref) => {
    try {
      return [`<a href="${esc(refUrl(ref))}">${esc(ref)}</a>`];
    } catch {
      // A malformed ref may still exist in an older or ungated in-memory
      // record. Keep the session visible and omit only the invalid link.
      return [];
    }
  });
  const refs = renderedRefs.length ? ` · ${renderedRefs.join(" ")}` : "";
  // #1156: live records carry the session action set on the merged
  // registry identity; historical records render no controls.
  const badges = actionBadgesHtml(act?.actions);
  const controls =
    act?.identities && sessionRecordIsLive(r) ? sessionActionsHtml(r.id, r.source, { identities: act.identities }) : "";
  return `<div class="card session-record-line" id="${esc(r.id)}"><strong>${esc(r.label)}</strong> <span class="badge src">${esc(r.source)}</span> <span class="badge agent">${esc(r.agentLabel)}</span>${badges}<br><span class="muted session-record-meta">${esc(r.id)} · ${esc(r.status)}${r.model ? ` · ${esc(r.model)}` : ""} · started ${fmtDate(r.startedAt)} · last seen ${fmtDate(r.lastSeenAt)}${tokens}${parent}${refs}</span>${controls}</div>`;
}

// --- Quota view (#851) + Usage & billing hub (design 2026-10-25) -----------

const SOURCE_CLASS_LABELS: Record<string, string> = {
  api: "API read",
  console: "console read",
  manual: "manual read",
  estimate: "estimate",
  invoice: "invoice",
};

export function fmtMetricValue(v: number | null, unit: string): string {
  if (v == null) return "not measured";
  if (unit === "usd") return `$${v.toFixed(2)}`;
  if (unit === "cny") return `¥${v.toFixed(2)}`;
  if (unit === "gb") return `${v} GB`;
  if (unit === "pct") return `${v}%`;
  return v.toLocaleString();
}

/** The freshness stamp every hub card carries (§1.1): measured-at + source + age. */
export function freshnessStamp(measuredAt: string | null, sourceClass: string, cadenceDays: number): string {
  const f = freshness(measuredAt, cadenceDays);
  const cls = SOURCE_CLASS_LABELS[sourceClass] ?? sourceClass;
  if (!measuredAt)
    return `<span class="badge stale">no dated read</span> <span class="muted">${esc(cls)} · ${esc("never measured")}</span>`;
  const age = f.ageDays == null ? "" : f.ageDays === 0 ? " · today" : ` · ${f.ageDays}d ago`;
  const stale = f.stale ? ` <span class="badge stale">stale — as of ${esc(measuredAt.slice(0, 10))}</span>` : "";
  return `<span class="muted">measured ${esc(measuredAt.slice(0, 10))}${age} · ${esc(cls)}</span>${stale}`;
}

function metricRow(m: UsageMetric): string {
  const pct = m.percent;
  const meter =
    pct != null
      ? `<meter min="0" max="100" value="${Math.min(100, pct).toFixed(0)}"></meter> ${pct.toFixed(0)}% · `
      : "";
  const limit = m.limit == null ? "" : ` / ${fmtMetricValue(m.limit, m.unit)}`;
  return `<div>${esc(m.label)}: ${meter}${fmtMetricValue(m.used, m.unit)}${limit}${m.note ? ` <span class="muted">— ${esc(m.note)}</span>` : ""}</div>`;
}

function attentionStripHtml(usage: UsageView): string {
  if (!usage.attention.length) {
    return `<div class="card"><strong>Needs attention</strong> — no quota or credential alerts. <span class="muted">(Bubble rules: any metric over 80% of its limit, or projected unbilled PAYG over $5.)</span></div>`;
  }
  const rows = usage.attention
    .map(
      (f) =>
        `<div class="flag${f.kind === "payg-unbilled" ? " payg" : ""}"><strong>${esc(f.label)}</strong> — ${esc(f.detail)} <a href="${esc(f.href)}">details</a></div>`,
    )
    .join("");
  return `<h2>Needs attention</h2>${rows}`;
}

/**
 * The DeepSeek PAYG reserve's balance, from the snapshot store (#1052):
 * the newest unit "cny" record. Shown in ¥ with its own measured-at —
 * the yuan figure as read, never converted to dollars.
 */
function paygBalanceHtml(usage: UsageView): string {
  const recs = (usage.history["deepseek"] ?? []).filter((r) => r.unit === "cny" && typeof r.value === "number");
  if (!recs.length) return "";
  const last = recs[recs.length - 1];
  return `<div><strong>DeepSeek PAYG reserve balance</strong>: <strong>¥${(last.value as number).toFixed(2)}</strong><br><span class="muted">reserve only — never a routine route · measured ${esc(last.ts.slice(0, 10))} · ${esc(last.observedBy)}</span></div>`;
}

function paygCardHtml(usage: UsageView): string {
  const balance = paygBalanceHtml(usage);
  const balanceBlock = balance ? `<div>${balance}</div>` : "";
  if (!usage.payg.length) {
    return `<div class="card"><div><strong>PAYG dollars</strong><br><span class="muted">pay-as-you-go agent spend</span></div><div class="muted">No metered spill recorded in local records.</div>${balanceBlock}</div>`;
  }
  const lines = usage.payg
    .map((l: PaygLine) => {
      const figures =
        l.accruedUsd != null
          ? `$${l.accruedUsd.toFixed(2)} accrued${l.collectedUsd != null ? ` · $${l.collectedUsd.toFixed(2)} collected` : ""} · $${(l.unbilledUsd ?? 0).toFixed(2)} unbilled`
          : l.estimate
            ? `counted $${countedEstimate(l.estimate).toFixed(2)} as used (1.2 × high) · estimate $${l.estimate.low.toFixed(2)}–$${l.estimate.high.toFixed(2)} ${esc(l.estimate.period)}`
            : "not estimable from local records";
      return `<div><strong>${esc(l.label)}</strong>: ${figures}<br><span class="muted">${esc(l.note)}</span><br>${freshnessStamp(l.measuredAt, l.sourceClass, 7)}</div>`;
    })
    .join("");
  const total = usage.payg.reduce((sum, l) => sum + (l.unbilledUsd ?? 0), 0);
  return `<div class="card"><div><strong>PAYG dollars</strong><br><span class="muted">metered spill, counted per Decision 004 — estimates at 1.2 × high</span><br><span class="badge ${total > 5 ? "near" : "healthy"}">$${total.toFixed(2)} projected unbilled</span></div><div>${lines}${balanceBlock}</div></div>`;
}

function serviceCardHtml(svc: SaasService, history: SnapshotRecord[]): string {
  const f = freshness(svc.measuredAt, svc.cadenceDays);
  const highlights = svc.metrics.filter((m) => m.highlight);
  const metrics = highlights.length
    ? highlights.map(metricRow).join("")
    : `<div class="muted">${esc(svc.usageNote ?? "No highlight metrics recorded.")}</div>`;
  const billed = svc.billedUsd != null ? `<div>billed this period: $${svc.billedUsd.toFixed(2)}</div>` : "";
  const payg = svc.paygEstimate
    ? `<div>PAYG estimate: counted $${countedEstimate(svc.paygEstimate).toFixed(2)} (${esc(svc.paygEstimate.period)}; range $${svc.paygEstimate.low.toFixed(2)}–$${svc.paygEstimate.high.toFixed(2)} in detail)</div>`
    : svc.paygNote
      ? `<div class="muted">PAYG: ${esc(svc.paygNote)}</div>`
      : "";
  const conflict = svc.conflicts.length
    ? ` <a class="badge conflict" href="/quota/service/${esc(svc.id)}" title="Sources disagree — the dated history is on the detail page">conflict</a>`
    : "";
  const pace = paceLineHtml(svc.id, history);
  return `<div class="card${f.stale ? " stale" : ""}"><div><strong><a href="/quota/service/${esc(svc.id)}">${esc(svc.name)}</a></strong>${svc.status !== "active" ? ` <span class="badge src">${esc(svc.status)}</span>` : ""}${conflict}<br><span class="muted">${esc(svc.plan)}</span><br>${freshnessStamp(svc.measuredAt, svc.sourceClass, svc.cadenceDays)}${svc.nextDate ? `<br><span class="muted">next: ${esc(svc.nextDate)}</span>` : ""}</div><div>${metrics}${billed}${payg}${pace}<div><a href="/quota/service/${esc(svc.id)}">show all →</a></div></div></div>`;
}

/** A one-line pace projection on a card, when the history supports one (§1.5). */
function paceLineHtml(serviceId: string, history: SnapshotRecord[]): string {
  const metricIds = [...new Set(history.filter((r) => r.unit === "pct").map((r) => r.metricId))];
  for (const metricId of metricIds) {
    const pace = projectPace(history, metricId);
    if (pace?.projectedExhaustionAt) {
      return `<div class="muted">pace (${esc(metricId)}): +${pace.slopePerDay} pts/day → projected to exhaust ${esc(pace.projectedExhaustionAt.slice(0, 10))}${pace.projectedPercentAtReset != null ? ` · ${pace.projectedPercentAtReset}% at reset` : ""}</div>`;
    }
  }
  return "";
}

function chargesStripHtml(usage: UsageView): string {
  if (!usage.charges.length) {
    return `<h2>Charges this period</h2><div class="card"><span class="muted">${esc(usage.chargesNote ?? "No charge evidence in the local records yet — rows appear as Plaid/Gmail receipt evidence is recorded.")}</span></div>`;
  }
  const rows = usage.charges
    .map(
      (c: ChargeRow) =>
        `<tr><td>${esc(c.at)}</td><td>${esc(c.label)}</td><td>$${c.amountUsd.toFixed(2)}</td><td>${esc(c.method)}</td><td class="muted">${esc(c.source)}</td></tr>`,
    )
    .join("");
  const total = usage.charges.reduce((sum, c) => sum + c.amountUsd, 0);
  return `<h2>Charges this period</h2><p class="muted">Evidence, not accounting — charges observed on cards/receipts in the local records (full balances stay in the finance tooling). Total observed: $${total.toFixed(2)}.</p><div class="tblwrap"><table class="table table-zebra"><caption>Charges this period</caption><tr><th scope="col">Date</th><th scope="col">Charge</th><th scope="col">Amount</th><th scope="col">Method</th><th scope="col">Evidence</th></tr>${rows}</table></div>`;
}

/** Lazy-refresh strip (#1171): a visible "refreshing" marker while a
 *  gatherer run is in flight, plus the manual refresh-now control. Both
 *  use the same trigger path; neither blocks the page on provider calls. */
function quotaRefreshHtml(
  refresh?: import("./quota-refresh.ts").RefreshStatus,
  generatedAt?: string | null,
): string {
  if (!refresh) return "";
  const marker = refresh.refreshing
    ? `<span class="badge refreshing" data-quota-refreshing="true">refreshing — the quota gatherer is running; reload for updated data</span>`
    : refresh.stale
      ? `<span class="muted">quota data is older than five minutes</span>`
      : "";
  // Manual refresh (#6125): the button POSTs the manual trigger route
  // (POST /api/quota/refresh -> quotaRefresher.manual -> the deployed
  // quota-gatherer.service, whose read is the single `opencode quota`
  // source), then polls /api/quota.json until quota-current.json's
  // generatedAt advances. Success is never faked: an advanced file whose
  // gatherer reported failure, a failed request, or a poll window with
  // no advance all surface as a failure message. The form remains the
  // no-JS path (POST /quota/refresh, same manual trigger, 303 back).
  return `<div class="card quota-refresh" data-quota-generated-at="${esc(generatedAt ?? "")}"${refresh.refreshing ? ' data-quota-refreshing="true"' : ""}><div><span data-quota-refresh-status aria-live="polite">${marker || '<span class="muted">quota data is fresh</span>'}</span></div><form method="post" action="/quota/refresh"><button type="submit" class="btn btn-primary btn-sm" data-quota-refresh-btn>Refresh quota</button></form></div>
<script>
(function () {
  var strip = document.querySelector(".quota-refresh[data-quota-generated-at]");
  if (!strip || strip.dataset.wired) return;
  strip.dataset.wired = "1";
  var btn = strip.querySelector("[data-quota-refresh-btn]");
  var status = strip.querySelector("[data-quota-refresh-status]");
  function say(text) { if (status) status.textContent = text; }
  var MAX_POLLS = 72; // 72 x 5s = 6 min, the gatherer's MAX_REFRESH_MS bound.
  function poll(baseline) {
    var tries = 0;
    var timer = setInterval(function () {
      tries++;
      fetch("/api/quota.json", { cache: "no-store" })
        .then(function (r) { return r.json(); })
        .then(function (data) {
          var gen = data && data.gatherer ? data.gatherer.generatedAt : null;
          if (gen && gen !== baseline) {
            clearInterval(timer);
            if (data.gatherer.ok === false) {
              say("refresh failed — the quota gatherer ran but reported an error; the previous data is still shown");
              if (btn) btn.disabled = false;
            } else {
              say("quota data updated — reloading…");
              window.location.reload();
            }
          } else if (tries >= MAX_POLLS) {
            clearInterval(timer);
            say("refresh did not complete — quota data did not advance, so the gatherer may have failed; the previous data is still shown");
            if (btn) btn.disabled = false;
          }
        })
        .catch(function () { /* a transient poll error keeps polling until the cap */ });
    }, 5000);
  }
  if (btn) {
    btn.addEventListener("click", function (ev) {
      ev.preventDefault();
      btn.disabled = true;
      say("refreshing — the quota gatherer is running…");
      fetch("/api/quota/refresh", { method: "POST" })
        .then(function (r) {
          if (!r.ok) throw new Error("the refresh request failed (" + r.status + ")");
          return r.json();
        })
        .then(function (res) {
          poll((res && res.generatedAt) || strip.getAttribute("data-quota-generated-at") || "");
        })
        .catch(function (err) {
          say("refresh failed — " + err.message);
          btn.disabled = false;
        });
    });
  }
  if (strip.getAttribute("data-quota-refreshing") === "true") {
    if (btn) btn.disabled = true;
    poll(strip.getAttribute("data-quota-generated-at") || "");
  }
})();
</script>`;
}

export function renderQuota(
  view: QuotaView,
  provenance: Provenance,
  usage?: UsageView,
  detail?: ProvenanceDetail,
  /** Lazy-refresh state (#1171): renders the "refreshing" marker and the
   *  manual refresh-now control. Absent means no request was made. */
  refresh?: import("./quota-refresh.ts").RefreshStatus,
): string {
  // "-direct" display rule (#6125): judged over every provider name/id
  // this surface renders — pool providers, pool ids, burn rows, the
  // model-choice pool lists, and credential-lifecycle provider names.
  // Display only; ids and keys below stay exactly as the view carries
  // them.
  const displayNames = providerDisplayNames([
    ...view.pools.map((p) => p.provider),
    ...view.pools.map((p) => p.id),
    ...(view.burn ?? []).map((b) => b.provider),
    ...(view.burn ?? []).map((b) => b.pool),
    ...(view.choices
      ? [...view.choices.hasHeadroom, ...view.choices.burnBeforeReset, ...view.choices.doNotStart]
      : []),
    ...(view.credentialLifecycle ?? []).map((c) => c.providerName),
  ]);
  const display = (name: string) => displayNames.get(name) ?? name;
  const credentialLifecycle = view.credentialLifecycle?.length
    ? `<h2>Credential lifecycle</h2><div class="card">${view.credentialLifecycle
        .map((credential) => {
          const expiry = credential.expiresAt ? ` · expires ${fmtDate(credential.expiresAt)}` : " · expiry unknown";
          const reason = credential.reason ? ` · ${esc(credential.reason)}` : "";
          const action = credential.userAction
            ? ` · <strong>user action:</strong> ${esc(credential.userAction.title)}`
            : "";
          return `<div><strong>${esc(display(credential.providerName))} · ${esc(credential.label)}</strong> <span class="badge ${esc(credential.state)}">${esc(credential.state)}</span>${expiry}${reason}${action}</div>`;
        })
        .join("")}</div>`
    : "";
  // #6671: provider-auth keeper problems render as their own card —
  // a dying provider session (or a keeper that stopped reporting)
  // must be visible on the board, not only in a state file. All-ok
  // keepers render nothing here (no alert, no noise).
  const providerAuthProblems = (view.providerAuth ?? []).filter((a) => a.reporting !== "ok");
  const providerAuthSection = providerAuthProblems.length
    ? `<h2>Provider auth</h2><div class="card">${providerAuthProblems
        .map((a) => {
          const detail =
            a.reporting === "attention"
              ? `${esc(a.alert ?? a.status ?? "attention")}${a.alertSince ? ` · since ${fmtDate(a.alertSince)}` : ""}${a.sessionExpiresAt ? ` · session expires ${fmtDate(a.sessionExpiresAt)}` : ""}${a.lastGoodAt ? ` · last good ${fmtDate(a.lastGoodAt)}` : ""}`
              : a.reporting === "stale"
                ? `keeper not reporting · last check ${fmtDate(a.checkedAt)}`
                : a.reporting === "invalid"
                  ? "keeper not reporting · state file invalid"
                  : "keeper not reporting · no state file";
          return `<div><strong>${esc(a.provider)}</strong> <span class="badge error">${esc(a.reporting)}</span> <span class="muted">${detail}</span></div>`;
        })
        .join("")}</div>`
    : "";
  const pools = view.pools
    .map((p) => {
      const windows = p.windows
        .map((w) => {
          const pct = windowPercent(w);
          const used = w.unit === "usd" ? `$${w.used.toFixed(2)}` : w.used.toLocaleString();
          const limit = w.limit == null ? "∞" : w.unit === "usd" ? `$${w.limit.toFixed(2)}` : w.limit.toLocaleString();
          return `<div>${esc(w.name)}: <meter min="0" max="100" value="${pct.toFixed(0)}"></meter> ${pct.toFixed(0)}% · ${used}/${limit} ${esc(w.unit)} · resets ${fmtDate(w.resetsAt)}</div>`;
        })
        .join("");
      const sessions = (p.activeSessions ?? [])
        .map(
          (s) =>
            `<div class="muted">burning: ${esc(s.session)} (${esc(s.model)}) ${s.tokens.toLocaleString()} tok</div>`,
        )
        .join("");
      const credRows = (p.credentials ?? [])
        .map((c) => {
          const wins = c.windows.map((w) => `${esc(w.name)} ${windowPercent(w).toFixed(0)}%`).join(" \u00b7 ");
          const state =
            c.freshness.state === "ok"
              ? ""
              : ` \u00b7 ${esc(c.freshness.state)}${c.freshness.error ? ` \u00b7 ${esc(c.freshness.error)}` : ""}`;
          return `<div class="muted">credential ${esc(c.label)}: ${wins || esc(c.freshness.state)}${state}</div>`;
        })
        .join("");
      const fresh = p.freshness
        ? `<div class="muted">gatherer · ${esc(p.freshness.state)} · measured ${fmtDate(p.freshness.measuredAt)} · ${esc(p.freshness.source)}${p.freshness.error ? ` · ${esc(p.freshness.error)}` : ""}</div>`
        : "";
      return `<div class="card pool"><div><strong>${esc(display(p.provider))}</strong><br><span class="muted">${esc(p.plan)}</span><br><span class="badge ${esc(p.state)}">${esc(p.state)}${p.wallLiftsAt ? ` · lifts ${fmtDate(p.wallLiftsAt)}` : ""}</span>${p.burnTokensPerMin ? ` <span class="muted">${p.burnTokensPerMin} tok/min</span>` : ""}</div><div>${windows}${credRows}${sessions}${fresh}${p.note ? `<div class="muted">${esc(p.note)}</div>` : ""}</div></div>`;
    })
    .join("");
  const burn = view.burn
    .map(
      (b) =>
        `<tr><td>${esc(display(b.provider))}</td><td>${esc(display(b.pool))}</td><td>${fmtDate(b.from)}</td><td>${fmtDate(b.to)}</td><td>${b.tokens.toLocaleString()}</td></tr>`,
    )
    .join("");
  const choices = (list: string[]) => (list.length ? list.map((id) => esc(display(id))).join(", ") : "—");
  const burnAndChoices = `<h2>Burn between resets</h2><div class="tblwrap"><table class="table table-zebra"><caption>Burn between resets</caption><tr><th scope="col">Provider</th><th scope="col">Pool</th><th scope="col">From</th><th scope="col">To</th><th scope="col">Tokens</th></tr>${burn || '<tr><td colspan="5" class="muted">none</td></tr>'}</table></div>
<h2>Model choice</h2><div class="tblwrap"><table class="table table-zebra"><caption>Model choice rules</caption><tr><th scope="col">Rule</th><th scope="col">Pools</th></tr>
<tr><td>Headroom for a new session</td><td>${choices(view.choices.hasHeadroom)}</td></tr>
<tr><td>Burn-it-before-reset (unused headroom)</td><td>${choices(view.choices.burnBeforeReset)}</td></tr>
<tr><td>Do not start a second session</td><td>${choices(view.choices.doNotStart)}</td></tr></table></div>`;
  if (!usage) {
    return layout(
      "Quota + throughput",
      `<h1>Quota + throughput</h1><p class="muted">All pooled quotas, resets, and burn rates. Machine-first: the same data is at <a href="/api/quota.json">/api/quota.json</a>.</p>
${credentialLifecycle}
${providerAuthSection}
${quotaRefreshHtml(refresh, view.gatherer?.generatedAt ?? null)}
<h2>Pools</h2>${pools}
${burnAndChoices}`,
      provenance,
      "/quota",
    );
  }
  const groupOrder: (keyof typeof SAAS_GROUP_LABELS)[] = [
    "database",
    "object-storage",
    "compute",
    "dev-platform",
    "domains",
    "account-tooling",
  ];
  const groups = groupOrder
    .map((g) => {
      const svcs = usage.services.filter((s) => s.group === g);
      const cards = svcs.map((s) => serviceCardHtml(s, usage.history[s.id] ?? [])).join("");
      const domainsCard =
        g === "domains"
          ? usage.domains
            ? `<div class="card"><div><strong><a href="/dns">Domains</a></strong><br><span class="muted">Porkbun register — summary only; the full register lives on the DNS page</span></div><div><div>renewal exposure: $${usage.domains.totalAnnualUsd.toFixed(2)}/yr across ${usage.domains.domainCount} domains</div><div>next renewal: ${esc(usage.domains.nextRenewal ?? "—")}</div><div><a href="/dns">the full register →</a></div></div></div>`
            : `<div class="card"><span class="muted">Domains: the register has not reached the feed yet — see <a href="/dns">/dns</a>.</span></div>`
          : "";
      const subs =
        g === "account-tooling" && usage.subscriptions.length
          ? `<div class="card"><div><strong>Subscriptions (flat)</strong><br><span class="muted">renewal-only rows — no usage meter</span></div><div>${usage.subscriptions
              .map(
                (s) =>
                  `<div>${esc(s.name)}: ${esc(s.plan)}${s.renewsAt ? ` · renews ${esc(s.renewsAt)}` : " · renewal date unknown"}</div>`,
              )
              .join("")}</div></div>`
          : "";
      if (!cards && !domainsCard && !subs) return "";
      return `<h3>${esc(SAAS_GROUP_LABELS[g])}</h3><div class="svcgrid">${cards}${domainsCard}${subs}</div>`;
    })
    .join("");
  return layout(
    "Usage & billing",
    `<h1>Usage &amp; billing</h1><p class="muted">Token quotas, PAYG dollars, and SaaS usage vs limits — from dated local records; every card carries its measured-at stamp, and stale cards dim. Machine-first: <a href="/api/quota.json">/api/quota.json</a>.</p>
${attentionStripHtml(usage)}
${credentialLifecycle}
${providerAuthSection}
${quotaRefreshHtml(refresh, view.gatherer?.generatedAt ?? null)}
<h2>Token quotas</h2>${pools}
${paygCardHtml(usage)}
<h2>SaaS &amp; services</h2>${groups || '<p class="muted">No services extracted yet.</p>'}
${chargesStripHtml(usage)}
<h2>Sources</h2><div class="card"><div>Research records behind the cards on this page that carry provenance — all in <code>~/workspace/ai-subscriptions/</code>:</div><div>· evidence baseline: <code>evidence-and-interpretation-baseline-2026-10-20.md</code> (dated official pricing wording + interpretation notes)</div><div>· master tracker: the tracker behind <code>quota-view.md</code> Part (b) — each card with a tracker row cites its Summary (b) row and service section</div><div>· candidate evaluations: <code>free-services-to-check.md</code></div><div class="muted">Per-service provenance (tracker row or its absence, baseline, quota-view section) is on each service's detail page.</div></div>
${burnAndChoices}`,
    provenance,
    "/quota",
    "",
    detail ? freshnessChips(detail, ["quota", "usage"]) : "",
  );
}

// --- Service detail view (hub v1.1, §1.5) -------------------------------------

/** An inline SVG history chart for one metric, with reset markers (§4). */
export function historyChartHtml(records: SnapshotRecord[], metricId: string): string {
  const recs = records
    .filter((r) => r.metricId === metricId && typeof r.value === "number")
    .sort((a, b) => a.ts.localeCompare(b.ts)) as (SnapshotRecord & { value: number })[];
  if (!recs.length) return "";
  const W = 640;
  const H = 130;
  const PAD = 28;
  const t0 = Date.parse(recs[0].ts);
  const t1 = Math.max(Date.parse(recs[recs.length - 1].ts), t0 + 1);
  const vmax = Math.max(...recs.map((r) => r.value), ...(recs.map((r) => r.limit ?? 0) as number[]), 1) * 1.1;
  const x = (ts: string) => PAD + ((Date.parse(ts) - t0) / (t1 - t0)) * (W - PAD - 8);
  const y = (v: number) => H - 18 - (v / vmax) * (H - 34);
  const markers = resetMarkers(records, metricId)
    .filter((m) => Date.parse(m.ts) >= t0 && Date.parse(m.ts) <= t1)
    .map(
      (m) =>
        `<line x1="${x(m.ts).toFixed(1)}" y1="8" x2="${x(m.ts).toFixed(1)}" y2="${H - 18}" stroke="#d29922" stroke-dasharray="3 3"><title>reset ${esc(m.resetsAt.slice(0, 10))}</title></line>`,
    )
    .join("");
  const limitLine = recs.find((r) => r.limit != null)
    ? `<line x1="${PAD}" y1="${y((recs.find((r) => r.limit != null) as SnapshotRecord).limit as number).toFixed(1)}" x2="${W - 8}" y2="${y((recs.find((r) => r.limit != null) as SnapshotRecord).limit as number).toFixed(1)}" stroke="#f85149" stroke-dasharray="2 4"></line>`
    : "";
  const points = recs
    .map(
      (r) =>
        `<circle cx="${x(r.ts).toFixed(1)}" cy="${y(r.value).toFixed(1)}" r="3" fill="#58a6ff"><title>${esc(r.ts.slice(0, 10))}: ${r.value} ${esc(r.unit)} (${esc(r.source)})</title></circle>`,
    )
    .join("");
  const line = recs.map((r, i) => `${i === 0 ? "M" : "L"}${x(r.ts).toFixed(1)},${y(r.value).toFixed(1)}`).join(" ");
  const first = recs[0];
  const last = recs[recs.length - 1];
  return `<div class="chart"><div class="muted">${esc(metricId)} · ${recs.length} snapshot${recs.length === 1 ? "" : "s"} · ${esc(first.ts.slice(0, 10))} → ${esc(last.ts.slice(0, 10))} · dashed amber = window reset, red = limit</div><svg width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" role="img" aria-label="history chart for ${esc(metricId)}">${markers}${limitLine}<path d="${line}" fill="none" stroke="#58a6ff" stroke-width="1.5"></path>${points}</svg></div>`;
}

function snapshotFreshnessStamp(history: SnapshotRecord[], cadenceDays: number, now: Date): string {
  const latest = history.reduce<SnapshotRecord | null>(
    (current, record) => (!current || Date.parse(record.ts) > Date.parse(current.ts) ? record : current),
    null,
  );
  if (!latest) {
    return '<p class="muted">No snapshot writer has landed a record for this service yet. The feed-ingest tick is the normal landing path; the §4 backfill is a one-shot recovery, not normal history startup.</p>';
  }
  const measuredAt = latest.ts.slice(0, 10);
  const f = freshness(measuredAt, cadenceDays, now);
  const age = f.ageDays == null ? " · age unknown" : f.ageDays === 0 ? " · today" : ` · ${f.ageDays}d ago`;
  const state = f.stale ? '<span class="badge stale">stale</span>' : '<span class="badge src">within cadence</span>';
  return `<p class="muted">Latest stored snapshot: <time datetime="${esc(latest.ts)}">${esc(measuredAt)}</time>${age} · ${cadenceDays}-day expected cadence · ${state}</p>`;
}

export function renderServiceDetail(
  svc: SaasService,
  history: SnapshotRecord[],
  provenance: Provenance,
  detail?: ProvenanceDetail,
  now = new Date(),
): string {
  const f = freshness(svc.measuredAt, svc.cadenceDays);
  const metricRows = svc.metrics.length
    ? svc.metrics
        .map(
          (m) =>
            `<tr><td>${esc(m.label)}${m.highlight ? ' <span class="badge src">highlight</span>' : ""}</td><td>${fmtMetricValue(m.used, m.unit)}</td><td>${m.limit == null ? "—" : fmtMetricValue(m.limit, m.unit)}</td><td>${m.percent != null ? `${m.percent.toFixed(0)}%` : "—"}</td><td class="muted">${esc(m.note ?? "")}</td></tr>`,
        )
        .join("")
    : '<tr><td colspan="5" class="muted">No metrics recorded for this service yet.</td></tr>';
  const payg = svc.paygEstimate
    ? `<h2>PAYG estimate</h2><div class="card">low $${svc.paygEstimate.low.toFixed(2)} / high $${svc.paygEstimate.high.toFixed(2)} ${esc(svc.paygEstimate.period)} — counted as used: <strong>$${countedEstimate(svc.paygEstimate).toFixed(2)}</strong> (1.2 × high, Decision 004).${svc.paygEstimate.note ? `<br><span class="muted">${esc(svc.paygEstimate.note)}</span>` : ""}</div>`
    : svc.paygNote
      ? `<h2>PAYG estimate</h2><div class="card"><span class="muted">${esc(svc.paygNote)} — no number is invented where the records carry no volume.</span></div>`
      : "";
  const metricIds = [...new Set(history.map((r) => r.metricId))];
  const charts = metricIds
    .map((id) => historyChartHtml(history, id))
    .filter(Boolean)
    .join("");
  const paces = metricIds
    .map((id) => ({ id, pace: projectPace(history, id) }))
    .filter((p) => p.pace)
    .map(
      (p) =>
        `<div>${esc(p.id)}: +${p.pace!.slopePerDay} pts/day over ${p.pace!.points} reads${p.pace!.projectedExhaustionAt ? ` → projected to exhaust ${esc(p.pace!.projectedExhaustionAt.slice(0, 10))}` : ""}${p.pace!.projectedPercentAtReset != null ? ` · projected ${p.pace!.projectedPercentAtReset}% at reset` : ""}</div>`,
    )
    .join("");
  const reads = svc.reads.length
    ? svc.reads.map((r) => `<div class="cmt"><span class="muted">${esc(r.at)}</span><br>${esc(r.text)}</div>`).join("")
    : '<p class="muted">No dated reads recorded.</p>';
  const conflicts = svc.conflicts.length
    ? `<h2>Conflicts (shown, not resolved)</h2>${svc.conflicts.map((c) => `<div class="reason">${esc(c)}</div>`).join("")}`
    : "";
  const body = `<p class="muted"><a href="/quota">← Usage &amp; billing</a></p>
<h1>${esc(svc.name)}</h1>
<div class="card${f.stale ? " stale" : ""}"><strong>${esc(svc.plan)}</strong> <span class="badge src">${esc(svc.status)}</span><br>${freshnessStamp(svc.measuredAt, svc.sourceClass, svc.cadenceDays)}<br><span class="muted">source: ${esc(svc.sourceNote)}${svc.nextDate ? ` · next: ${esc(svc.nextDate)}` : ""}${svc.billedUsd != null ? ` · billed this period: $${svc.billedUsd.toFixed(2)}` : ""}</span>${svc.usageNote ? `<br>${esc(svc.usageNote)}` : ""}</div>
<h2>Sources</h2><div class="card">${svc.sources ? `<div>quota-view section: <code>${esc(svc.sources.quotaViewSection)}</code> in <code>quota-view.md</code> Part (b)</div><div>tracker row: ${svc.sources.trackerRow ? `<code>Summary (b) — ${esc(svc.sources.trackerRow)}</code>` : (svc.status === "candidate" || svc.status === "no-account") ? "none — no Summary (b) row; candidate / not held (§1.4)" : "none — no Summary (b) row"}</div><div>evidence baseline: <code>${esc(svc.sources.baselineDoc)}</code></div>` : `<div class="muted">source: ${esc(svc.sourceNote)}</div>`}</div>
<h2>All metrics</h2><div class="tblwrap"><table class="table table-zebra"><caption>All metrics</caption><tr><th scope="col">Metric</th><th scope="col">Used</th><th scope="col">Limit</th><th scope="col">%</th><th scope="col">Note</th></tr>${metricRows}</table></div>
${payg}
<h2>Snapshot history</h2>${snapshotFreshnessStamp(history, svc.cadenceDays, now)}${charts || ""}
${paces ? `<h2>Pace projections</h2><div class="card">${paces}</div>` : ""}
${conflicts}
<h2>Dated source reads</h2>${reads}
<p class="muted">Machine-first: <a href="/api/quota/service.json?id=${encodeURIComponent(svc.id)}">/api/quota/service.json?id=${esc(svc.id)}</a></p>`;
  return layout(
    `${svc.name} · usage`,
    body,
    provenance,
    "/quota",
    "",
    detail ? freshnessChips(detail, ["quota", "usage"]) : "",
  );
}

// --- Attention view (#852, tiered in slice 2) -------------------------------

function sessionCard(
  e: TieredEntry,
  children: Map<string, TieredEntry[]>,
  depth = 0,
  counts?: Map<string, number | CommentKindCounts>,
  act?: {
    actionsByObject?: Map<string, DashAction[]>;
    identities?: AuthorIdentity[];
    commentsByObject?: Map<string, DashComment[]>;
    commentOpts?: CommentFormOpts;
  },
): string {
  const src = SESSION_SOURCE_LABELS[sourceOf(e)];
  const sub =
    e.parentId && depth === 0
      ? ` <span class="badge src">sub-session${e.agent ? ` · ${esc(e.agent)}` : ""} ↳ ${esc(e.parentId)}</span>`
      : e.agent
        ? ` <span class="badge src">${esc(e.agent)}</span>`
        : "";
  const tierBadge =
    e.tier === "needs-drew"
      ? `<span class="badge drew">Needs User</span>`
      : e.tier === "needs-orchestrator"
        ? `<span class="badge orch">Needs Orchestrator</span>`
        : "";
  const evidence = e.supervisionNote ? `${e.evidence} Keeper: ${e.supervisionNote}` : e.evidence;
  const reason = e.drewReason ? `<div class="reason"><strong>Why you:</strong> ${esc(e.drewReason)}</div>` : "";
  const orch = e.orchestratorNote ? `<div class="orch-note">${esc(e.orchestratorNote)}</div>` : "";
  const kids = children.get(e.sessionId) ?? [];
  const kidsHtml = kids.length
    ? `<details class="kids"><summary>${kids.length} sub-session${kids.length === 1 ? "" : "s"}</summary>${kids.map((k) => sessionCard(k, children, depth + 1, counts, act)).join("")}</details>`
    : "";
  const cmt = counts ? commentBadge(sessionObjectRef(e.sessionId), counts) : "";
  // #1156 + #1152: session actions (poke/answer/stop by source support)
  // and, for tiered entries, the triage set — session-derived items
  // stay actionable even when the tier source is down (the session
  // feed is live), so no disabledReason ever applies here.
  const objectRef = sessionObjectRef(e.sessionId);
  const own = act?.actionsByObject?.get(objectRef);
  const badges = actionBadgesHtml(own);
  const cs = cardState(own);
  let controls = "";
  if (act?.identities) {
    const sessionBox = sessionActionsHtml(e.sessionId, sourceOf(e), { identities: act.identities });
    const triageBox =
      e.tier === "needs-drew" || e.tier === "needs-orchestrator"
        ? triageActionsHtml(objectRef, e.tier, { identities: act.identities }, "", cs)
        : "";
    controls = `${sessionBox}${triageBox}`;
  }
  // #1445/#1446: inline detail (state + full action history) and the
  // per-card comment box ride the session card too.
  const detail = `<details class="carddetail"><summary>Details &amp; history</summary><dl><dt>Session</dt><dd><code>${esc(e.sessionId)}</code></dd><dt>State</dt><dd>${esc(STATE_LABELS[e.state] ?? e.state)}</dd><dt>Last activity</dt><dd>${esc(fmtDate(e.lastActivityAt))}</dd><dt>Ref</dt><dd><code>${esc(objectRef)}</code></dd></dl>${actionHistoryHtml(own)}</details>`;
  const commentBox =
    act?.identities && act.commentOpts
      ? cardCommentHtml(objectRef, undefined, act.commentsByObject?.get(objectRef), act.commentOpts)
      : "";
  return `<div class="card${cs.acked ? " acked" : ""}" id="${cardDomId(objectRef)}"><div class="attention-card-header"><strong><a href="/sessions?id=${encodeURIComponent(e.sessionId)}#${encodeURIComponent(e.sessionId)}">${esc(e.title ?? e.sessionId)}</a></strong> <span class="badge src">${esc(src)}</span> <span class="badge ${esc(e.state)}">${esc(STATE_LABELS[e.state])}</span>${tierBadge}${cs.acked ? ` <span class="badge acked">Acknowledged</span>` : ""}${sub}${cmt}${badges}</div><span class="muted">${esc(e.sessionId)} · ${esc(e.directory ?? "—")}${e.branch ? ` · ${esc(e.branch)}` : ""} · ${esc(e.model ?? "—")}${e.provider ? ` / ${esc(e.provider)}` : ""} · last activity ${fmtDate(e.lastActivityAt)} · ${esc(e.messageCount ?? "—")} msgs${e.outcome ? ` · outcome: ${esc(e.outcome)}` : ""}</span><br>${esc(evidence)}${reason}${orch}${controls}${detail}${commentBox}${kidsHtml}</div>`;
}

/** A #899 tier item that is not a live session — a decision, task, or gap. */
function tierItemCard(
  item: TierItem,
  whyLabel: string,
  counts?: Map<string, number | CommentKindCounts>,
  act?: {
    actions?: DashAction[];
    identities?: AuthorIdentity[];
    disabledReason?: string;
    comments?: DashComment[];
    commentOpts?: CommentFormOpts;
  },
): string {
  const blocked = item.blockedBy?.length
    ? `<br><strong>Blocked by:</strong> ${item.blockedBy
        .map((d) => esc(`${d.task} (${d.status}${d.workerLabel ? `, ${d.workerLabel}` : ""})`))
        .join("; ")}`
    : "";
  const question = item.question ? `<br><span class="q">“${esc(item.question)}”</span>` : "";
  const action = item.recommendedAction
    ? `<br><span class="muted">Recommended: ${esc(item.recommendedAction)}</span>`
    : "";
  // A tier item's ref is only sometimes a GitHub ref: tracked-goal items
  // carry their goal id (e.g. "goal_…"), and refUrl throws on those — one
  // such item 500ed the whole Needs-you render (phase-b inventory,
  // 2026-11-05). Non-GitHub refs render as plain code, never a link.
  const refLink = item.ref
    ? isValidRef(item.ref)
      ? `<a href="${esc(refUrl(item.ref))}">${esc(item.ref)}</a>`
      : `<code>${esc(item.ref)}</code>`
    : "";
  const ref = item.ref ? `<br>${refLink}` : "";
  const cmt = counts ? commentBadge(tierItemObjectRef(item), counts) : "";
  const badges = actionBadgesHtml(act?.actions);
  // #1291: on Needs User cards, the item's own contract renders — the
  // why-class badge in the header, the summary / if-ignored /
  // delegation / proposal / inability block in the body, and the
  // options / survey / delegation / dismiss forms in the actbox.
  const isUserTier = whyLabel === "Why you";
  const objectRef = tierItemObjectRef(item);
  const cs = cardState(act?.actions);
  const extBody = isUserTier ? needsUserBodyHtml(item, act?.actions) : "";
  const extForms = isUserTier ? needsUserFormsHtml(item, objectRef, Date.now(), cs) : "";
  // #1152: the triage control set. Section-derived items disable with
  // the reason when the tier source is absent/invalid (#1139 state) —
  // the item may not exist in the publisher's current view.
  const controls = act?.identities
    ? triageActionsHtml(
        objectRef,
        isUserTier ? "needs-drew" : "needs-orchestrator",
        {
          identities: act.identities,
          ...(act.disabledReason ? { disabledReason: act.disabledReason } : {}),
        },
        extForms,
        cs,
      )
    : "";
  // #1445: the title itself links to the GitHub issue when the card
  // carries a gh: ref, and every card expands inline to its full
  // detail — source, state, since/lastSeenAt, horizon, blockedBy,
  // the question payload, and the recorded action history.
  const titleHtml = item.ref && isValidRef(item.ref) ? `<a href="${esc(refUrl(item.ref))}">${esc(item.title)}</a>` : esc(item.title);
  const questionPayload = item.questions?.length
    ? `<dt>Questions</dt><dd>${item.questions
        .map(
          (q) =>
            `${esc(q.prompt)} <span class="muted">(covers: ${esc(q.covers)}${q.required ? "" : "; optional"})</span>`,
        )
        .join("<br>")}</dd>`
    : item.question
      ? `<dt>Question</dt><dd>“${esc(item.question)}”</dd>`
      : "";
  const detail = `<details class="carddetail"><summary>Details &amp; history</summary><dl><dt>Source</dt><dd>${esc(item.source)} · ${esc(item.kind)}${item.state ? ` · state ${esc(item.state)}` : ""}</dd><dt>Since</dt><dd>${esc(fmtDate(item.since))}</dd><dt>Last seen</dt><dd>${esc(fmtDate(item.lastSeenAt))}</dd>${item.horizon ? `<dt>Horizon</dt><dd>${esc(item.horizon)}</dd>` : ""}<dt>Ref</dt><dd><code>${esc(objectRef)}</code>${item.ref ? ` · ${refLink}` : ""}</dd>${item.blockedBy?.length ? `<dt>Blocked by</dt><dd>${item.blockedBy.map((d) => esc(`${d.task} (${d.status}${d.workerLabel ? `, ${d.workerLabel}` : ""})`)).join("; ")}</dd>` : ""}${questionPayload}</dl>${actionHistoryHtml(act?.actions)}</details>`;
  const commentBox =
    act?.identities && act.commentOpts ? cardCommentHtml(objectRef, item.ref, act.comments, act.commentOpts) : "";
  return `<div class="card${cs.acked ? " acked" : ""}" id="${cardDomId(objectRef)}"><div class="attention-card-header"><span class="badge src">${esc(item.source)}</span> <span class="badge">${esc(item.kind)}</span>${isUserTier ? whyUserBadge(item) : ""}${cs.acked ? ` <span class="badge acked">Acknowledged</span>` : ""} <strong>${titleHtml}</strong>${cmt}${badges}</div><br><strong>${esc(whyLabel)}:</strong> ${esc(item.reason)}${question}${blocked}${action}${extBody}${ref}${controls}${detail}${commentBox}</div>`;
}

export interface AttentionAct {
  actionsByObject?: Map<string, DashAction[]>;
  identities?: AuthorIdentity[];
  commentsByObject?: Map<string, DashComment[]>;
  commentOpts?: CommentFormOpts;
  /** #1456: the effective Needs-you card order + its pending state. */
  needsOrder?: NeedsOrder;
  needsOrderPending?: boolean;
}

interface StripEntry {
  objectRef: string;
  title?: string;
  cs: CardState;
}

/** One suppressed card in a strip (#1443/#1450): what happened, when it
 *  lifts, and the way back — Unsnooze for a snooze (a superseding
 *  snooze at "now"), Restore for a dismiss / mark-done (a restore
 *  action superseding the suppressor). */
function stripCardHtml(entry: StripEntry, identities: AuthorIdentity[] | undefined): string {
  const sup = entry.cs.suppressed!;
  const refLink = `<a href="/discuss/${encodeURIComponent(entry.objectRef)}">${entry.title ? esc(entry.title) : `<code>${esc(entry.objectRef)}</code>`}</a>`;
  if (sup.kind === "snoozed") {
    const a = sup.action;
    const stateLine =
      a.state === "new"
        ? `⏳ sent · pending — snooze until ${esc(fmtDate(sup.until))}`
        : `applied by ${esc(a.appliedByRun ?? "a fleet run")} — snoozed until ${esc(fmtDate(sup.until))}`;
    const unsnooze =
      identities !== undefined
        ? actboxHtml(
            { identities },
            aformHtml({
              kind: "triage",
              verb: "snooze",
              objectRef: entry.objectRef,
              supersedes: a.id,
              inner: `<input type="hidden" name="untilPreset" value="${new Date().toISOString()}"><button type="submit">Unsnooze now</button>`,
            }),
          )
        : "";
    return `<div class="card stripcard" id="${cardDomId(entry.objectRef)}">💤 ${refLink} <span class="badge act-snoozed">snoozed until ${esc(fmtDate(sup.until))}</span><br><span class="muted">${stateLine}${a.resultNote ? ` — ${esc(a.resultNote)}` : ""}</span>${unsnooze}</div>`;
  }
  const a = sup.action;
  const label = sup.kind === "done" ? "✓ marked done by you" : "dismissed by you";
  const stateLine =
    a.state === "new"
      ? "⏳ sent · pending"
      : a.state === "applied"
        ? `applied by ${esc(a.appliedByRun ?? "a fleet run")}`
        : a.state;
  const restore =
    identities !== undefined
      ? actboxHtml(
          { identities },
          aformHtml({
            kind: "triage",
            verb: "restore",
            objectRef: entry.objectRef,
            supersedes: a.id,
            inner: `<button type="submit">Restore to the queue</button><span class="why">Records a restore that supersedes the ${sup.kind === "done" ? "mark-done" : "dismiss"} — the card returns with its history intact.</span>`,
          }),
        )
      : "";
  return `<div class="card stripcard" id="${cardDomId(entry.objectRef)}">${refLink} <span class="badge act-applied">${label}</span><br><span class="muted">${stateLine}${a.payload.reason ? ` — reason: ${esc(a.payload.reason)}` : ""}${a.resultNote ? ` · ${esc(a.resultNote)}` : ""} · ${esc(relTime(a.createdAt))}</span>${restore}</div>`;
}

const HORIZON_RANK: Record<string, number> = { today: 0, "3d": 1, week: 2, month: 3, quarter: 4 };

/** #1456: the order control for the Needs-you list — a preference
 *  action; the render applies the saved order immediately. */
function needsOrderHtml(act: AttentionAct | undefined): string {
  if (!act?.identities) return "";
  const order = act.needsOrder ?? "urgency";
  const labels: Record<NeedsOrder, string> = {
    urgency: "Urgency (orchestrator's order)",
    newest: "Newest first",
    oldest: "Oldest first",
    deadline: "Deadline (horizon)",
  };
  const options = (Object.keys(labels) as NeedsOrder[])
    .map((o) => `<option value="${o}"${o === order ? " selected" : ""}>${labels[o]}</option>`)
    .join("");
  const form = aformHtml({
    kind: "preference",
    verb: "set-needs-order",
    objectRef: NEEDS_ORDER_OBJECT_REF,
    inner: `<label>Card order <select name="needsOrder">${options}</select></label><button type="submit">Save order</button>${act.needsOrderPending ? `<span class="badge act-pending">⏳ change pending — the orchestrator's canonical order follows on its next pass</span>` : ""}`,
  });
  return `<div class="orderctl">Needs-you list order: <strong>${esc(labels[order])}</strong>${actboxHtml({ identities: act.identities }, form)}</div>`;
}

/** #1277: the per-card bulk-triage pick for one queue card. Renders
 * only when the action surface is up (identities present); a card
 * whose own controls are disabled (stale tier source) gets a disabled
 * pick, so it cannot be submitted against an unconfirmed item. */
function bulkPickHtml(
  objectRef: string,
  title: string,
  act: AttentionAct | undefined,
  disabledReason?: string,
): string {
  if (!act?.identities) return "";
  return `<div class="bulkpickline"><label><input type="checkbox" class="bulkpick" data-object="${esc(objectRef)}" data-title="${esc(title)}"${disabledReason ? " disabled" : ""}> Select for bulk triage</label></div>`;
}

function bulkSnoozeOptions(): string {
  const now = new Date();
  const tonight = new Date(now);
  tonight.setHours(23, 59, 0, 0);
  const tomorrow = new Date(now);
  tomorrow.setDate(tomorrow.getDate() + 1);
  tomorrow.setHours(9, 0, 0, 0);
  const nextWeek = new Date(now);
  nextWeek.setDate(nextWeek.getDate() + 7);
  nextWeek.setHours(9, 0, 0, 0);
  return `<option value="${tonight.toISOString()}">until tonight</option><option value="${tomorrow.toISOString()}">until tomorrow 9:00</option><option value="${nextWeek.toISOString()}">until next week</option>`;
}

/** #1277: the bulk triage bar for the Needs-you queues — one verb
 * applied to every picked card, with a review step that lists every
 * selected item + the chosen action before anything is submitted.
 * Submission goes to POST /api/actions/batch (BULK_SCRIPT below);
 * each item lands as its own attributed action record. */
function bulkBarHtml(act: AttentionAct | undefined): string {
  if (!act?.identities) return "";
  return `<div class="bulkbar" id="bulkbar"><strong>Bulk triage</strong>${authorSelectHtml(act.identities)}<label>Action <select name="bulkVerb"><option value="acknowledge">acknowledge</option><option value="snooze">snooze</option><option value="escalate">escalate</option><option value="delegate">delegate</option></select></label><span class="bulk-snooze-fields" hidden><label>Snooze <select name="bulkUntil">${bulkSnoozeOptions()}</select></label></span><span class="bulk-tier-fields" hidden><label>Move to <select name="bulkTier"><option value="needs-drew">Needs User</option><option value="needs-orchestrator">Needs Orchestrator</option></select></label></span><span class="bulk-mode-fields" hidden><label>Delegation <select name="bulkMode"><option value="first-pass">first-pass</option><option value="auto">auto</option><option value="user">user (take it back)</option></select></label></span><label>Reason (optional) <input type="text" name="bulkReason"></label><label><input type="checkbox" class="bulk-all"> Select all visible</label><span class="bulkcount">0 selected</span><button type="button" class="bulk-review-btn" disabled>Review batch</button><div class="bulkreview" hidden><p class="bulkreview-summary"></p><ul class="bulkreview-list"></ul><button type="button" class="bulk-confirm-btn">Submit batch</button> <button type="button" class="bulk-cancel-btn">Back</button><div class="bulkresults"></div><div class="err bulkerr"></div></div></div>`;
}

/**
 * The client side of bulk triage (#1277): delegated listeners in the
 * COMMENT_SCRIPT/ACTION_SCRIPT pattern, so Datastar fragment swaps
 * keep working. Review builds nothing server-side; confirm POSTs one
 * action object per picked card to /api/actions/batch and renders
 * the per-item results — succeeded picks clear, failed picks stay
 * checked (their items remain in Needs-you) with the error named.
 */
const BULK_SCRIPT = `<script>${DASH_AFTER_JS}
(function () {
  if (window.__dashBulk) return;
  window.__dashBulk = true;
  // The reviewed batch, snapshotted at Review time (Codex review on
  // PR #4053): Submit sends exactly this envelope — never a reread of
  // live controls — and its per-item client tokens survive a lost
  // response, so a retry dedupes against anything already recorded
  // instead of appending duplicates. Any change to the picks or the
  // action controls after review invalidates the snapshot.
  var snapshot = null;
  var submitting = false;
  var boundBar = null;
  function picks() { return Array.prototype.slice.call(document.querySelectorAll("input.bulkpick:checked:not(:disabled)")); }
  function bar() { return document.getElementById("bulkbar"); }
  function invalidate() {
    if (submitting || !snapshot) return;
    snapshot = null;
    var b = bar(); if (!b) return;
    b.querySelector(".bulkreview").hidden = true;
    b.querySelector(".bulkerr").textContent = "Selection or action changed after review \u2014 review the batch again before submitting.";
  }
  function showReview(b, opts) {
    if (!snapshot) return;
    boundBar = b;
    var list = b.querySelector(".bulkreview-list");
    list.replaceChildren();
    snapshot.items.forEach(function (it) {
      var li = document.createElement("li");
      li.textContent = it.title + " \u2014 " + it.object;
      list.appendChild(li);
    });
    // Re-check the reviewed picks on the live cards (a fragment
    // refresh re-renders them unchecked; the snapshot is the truth).
    snapshot.items.forEach(function (it) {
      var pk = document.querySelector('input.bulkpick[data-object="' + CSS.escape(it.object) + '"]');
      if (pk && !pk.disabled) pk.checked = true;
    });
    var summary = b.querySelector(".bulkreview-summary");
    if (submitting) summary.textContent = "Submitting \u201c" + snapshot.text + "\u201d for " + snapshot.actions.length + " item" + (snapshot.actions.length === 1 ? "" : "s") + " \u2014 waiting for the result\u2026";
    else if (opts && opts.restored) summary.textContent = "Review restored after a page refresh: \u201c" + snapshot.text + "\u201d for " + snapshot.actions.length + " item" + (snapshot.actions.length === 1 ? "" : "s") + ". Submit retries exactly this reviewed batch.";
    else summary.textContent = "Apply \u201c" + snapshot.text + "\u201d to " + snapshot.actions.length + " item" + (snapshot.actions.length === 1 ? "" : "s") + "? Nothing is submitted yet \u2014 Submit sends exactly this reviewed list.";
    if (!opts || !opts.keepResults) b.querySelector(".bulkresults").replaceChildren();
    if (!opts || !opts.restored) b.querySelector(".bulkerr").textContent = "";
    b.querySelector(".bulk-confirm-btn").disabled = submitting;
    b.querySelector(".bulkreview").hidden = false;
    syncBar();
  }
  function syncBar() {
    var b = bar(); if (!b) return;
    var n = picks().length;
    b.querySelector(".bulkcount").textContent = n + " selected";
    b.querySelector(".bulk-review-btn").disabled = submitting || n === 0;
    var verb = b.querySelector("select[name=bulkVerb]").value;
    b.querySelector(".bulk-snooze-fields").hidden = verb !== "snooze";
    b.querySelector(".bulk-tier-fields").hidden = verb !== "escalate";
    b.querySelector(".bulk-mode-fields").hidden = verb !== "delegate";
  }
  document.addEventListener("change", function (ev) {
    var t = ev.target;
    if (!t || !t.classList) return;
    if (t.classList.contains("bulk-all")) {
      invalidate();
      document.querySelectorAll("input.bulkpick:not(:disabled)").forEach(function (pk) { pk.checked = t.checked; });
      syncBar();
    } else if (t.classList.contains("bulkpick") || t.name === "bulkVerb" || t.name === "bulkUntil" || t.name === "bulkTier" || t.name === "bulkMode") {
      invalidate();
      syncBar();
    }
  });
  document.addEventListener("input", function (ev) {
    var t = ev.target;
    if (t && t.name === "bulkReason") invalidate();
  });
  function describeAction(b) {
    var verb = b.querySelector("select[name=bulkVerb]").value;
    if (verb === "snooze") return "snooze until " + new Date(b.querySelector("select[name=bulkUntil]").value).toLocaleString();
    if (verb === "escalate") return "escalate to " + b.querySelector("select[name=bulkTier]").value;
    if (verb === "delegate") return "delegate (" + b.querySelector("select[name=bulkMode]").value + ")";
    return verb;
  }
  document.addEventListener("click", async function (ev) {
    var t = ev.target;
    if (!t || !t.classList) return;
    var b = bar(); if (!b) return;
    if (t.classList.contains("bulk-review-btn")) {
      if (submitting) return;
      var sel = picks(); if (!sel.length) return;
      var verb = b.querySelector("select[name=bulkVerb]").value;
      var authorRaw = String((b.querySelector("select[name=author]") || {}).value || "drew|drew|");
      var parts = authorRaw.split("|");
      var labelEl = b.querySelector('option[value="' + CSS.escape(authorRaw) + '"]');
      var author = { kind: parts[0], id: parts[1], label: labelEl ? labelEl.textContent : parts[1] };
      if (parts[2]) author.sessionId = parts[2];
      var payload = {};
      if (verb === "snooze") payload.until = b.querySelector("select[name=bulkUntil]").value;
      if (verb === "escalate") payload.tier = b.querySelector("select[name=bulkTier]").value;
      if (verb === "delegate") payload.mode = b.querySelector("select[name=bulkMode]").value;
      var reason = b.querySelector("input[name=bulkReason]").value.trim();
      if (reason) payload.reason = reason;
      var stamp = Date.now().toString(36) + "-" + Math.random().toString(36).slice(2, 8);
      var actions = sel.map(function (pk, i) {
        var a = { clientToken: "ct-bulk-" + stamp + "-" + i, kind: "triage", verb: verb, objectRef: pk.dataset.object, author: author, payload: payload };
        if (pk.dataset.object.indexOf("gh:") === 0) a.targetRef = pk.dataset.object.slice(3);
        return a;
      });
      snapshot = {
        actions: actions,
        text: describeAction(b),
        items: sel.map(function (pk) { return { object: pk.dataset.object, title: pk.dataset.title || pk.dataset.object }; }),
      };
      showReview(b, {});
    }
    if (t.classList.contains("bulk-cancel-btn")) {
      if (submitting) return;
      snapshot = null;
      b.querySelector(".bulkreview").hidden = true;
    }
    if (t.classList.contains("bulk-confirm-btn")) {
      if (submitting || !snapshot) return;
      var batch = snapshot;
      submitting = true;
      t.disabled = true;
      b.querySelector(".bulk-review-btn").disabled = true;
      var resultsEl = b.querySelector(".bulkresults");
      resultsEl.replaceChildren();
      var allResults = [];
      try {
        // The endpoint accepts at most 100 actions per request and
        // readJsonBody rejects bodies over 100,000 UTF-8 bytes, so the
        // SAME snapshot is packed greedily under BOTH limits and the
        // slices' per-item results are aggregated.
        var chunks = [];
        var cur = [];
        var encoder = new TextEncoder();
        var curBytes = 14; // UTF-8 bytes in the {"actions":[]} envelope
        batch.actions.forEach(function (a) {
          var bytes = encoder.encode(JSON.stringify(a)).byteLength + 1; // include a comma for headroom
          if (cur.length && (cur.length >= 100 || curBytes + bytes > 90000)) {
            chunks.push(cur);
            cur = [];
            curBytes = 14;
          }
          cur.push(a);
          curBytes += bytes;
        });
        if (cur.length) chunks.push(cur);
        for (var ci = 0; ci < chunks.length; ci++) {
          var res = await fetch("/api/actions/batch", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ actions: chunks[ci] }) });
          var body;
          try {
            body = await res.json();
          } catch (e) {
            // A 200 whose body never finished is an UNKNOWN outcome,
            // not an empty success: rethrow so the outer handler
            // keeps the snapshot (and its tokens) for an exact retry.
            if (res.ok) throw e;
            body = {};
          }
          if (!res.ok) throw new Error((body && body.error) || ("HTTP " + res.status));
          if (!body || !Array.isArray(body.results) || body.results.length !== chunks[ci].length) {
            throw new Error("batch response did not include per-item results for every action");
          }
          body.results.forEach(function (r) { allResults.push(r); });
        }
      } catch (e) {
        // No definitive result: the snapshot (and its client tokens)
        // is kept, so Submit retries the identical envelope and the
        // endpoint dedupes anything a lost response had recorded. If
        // a fragment refresh swapped the bar mid-flight, restore the
        // review UI on the LIVE bar first \u2014 the retry must never
        // be stranded on a detached element.
        var bLive = bar();
        if (bLive && bLive !== b) {
          showReview(bLive, { restored: true, keepResults: true });
          b = bLive;
        }
        submitting = false;
        b.querySelector(".bulkreview-summary").textContent = "Batch result unknown for \u201c" + batch.text + "\u201d \u2014 Submit retries exactly this reviewed batch.";
        b.querySelector(".bulkerr").textContent = "Batch result unknown: " + e.message + " \u2014 the reviewed batch is unchanged; Submit retries it exactly, and already-recorded items dedupe by client token.";
        var cf = b.querySelector(".bulk-confirm-btn");
        if (cf) cf.disabled = false;
        b.querySelector(".bulk-review-btn").disabled = picks().length === 0;
        return;
      }
      // Definitive results: the snapshot is spent. A further batch
      // needs a fresh review, which mints fresh tokens.
      snapshot = null;
      submitting = false;
      var bNow = bar();
      if (bNow && bNow !== b) {
        // The queues fragment was re-rendered mid-flight (e.g. a
        // single-card action refreshed it): write the results into
        // the LIVE bar, not the detached one.
        b = bNow;
        boundBar = bNow;
        resultsEl = b.querySelector(".bulkresults");
        resultsEl.replaceChildren();
        b.querySelector(".bulkreview").hidden = false;
        var cf2 = b.querySelector(".bulk-confirm-btn");
        if (cf2) cf2.disabled = true;
      }
      var failedRefs = {};
      var recorded = 0;
      var verb0 = batch.actions.length ? batch.actions[0].verb : "";
      allResults.forEach(function (r) {
        var line = document.createElement("div");
        if (r.ok) {
          recorded++;
          line.textContent = "\u2713 " + r.objectRef + " \u2014 recorded";
          // Update the accepted card in place (the __dashAfter
          // pattern): uncheck its pick, stamp the sent line, and
          // tombstone it for a snooze. This runs even when other
          // items failed \u2014 a partial batch gets no fragment
          // refresh, so failed picks and errors below stay put.
          var pk = document.querySelector('input.bulkpick[data-object="' + CSS.escape(r.objectRef) + '"]');
          if (pk) {
            pk.checked = false;
            var pickLine = pk.closest(".bulkpickline");
            var card = pickLine ? pickLine.nextElementSibling : null;
            if (card && card.classList.contains("card")) {
              var sent = document.createElement("div");
              sent.className = "sentline";
              sent.textContent = "\u2713 " + verb0 + " sent \u00b7 pending \u2014 the fleet applies it on its next pass";
              card.appendChild(sent);
              if (verb0 === "snooze") card.classList.add("tombstone");
            }
          }
        } else {
          line.textContent = "\u2717 " + (r.objectRef || "(unknown ref)") + " \u2014 " + r.error + " (still in Needs-you)";
          failedRefs[r.objectRef] = true;
        }
        resultsEl.appendChild(line);
      });
      var failed = allResults.length - recorded;
      b.querySelector(".bulkreview-summary").textContent = "Batch submitted: " + recorded + " recorded, " + failed + " failed. Change the selection and Review again for another batch.";
      syncBar();
      if (!failed) {
        window.dispatchEvent(new CustomEvent("dashrefresh"));
        if (!document.querySelector("[data-frag]")) location.reload();
      }
    }
  });
  // A queue fragment refresh (a Datastar swap \u2014 e.g. a single-card
  // action while a bulk batch is in flight or awaiting retry)
  // replaces #bulkbar wholesale. When a reviewed snapshot exists,
  // rebuild its review UI on the new bar, so the batch and its exact
  // retry survive the refresh instead of stranding on detached nodes.
  boundBar = bar();
  new MutationObserver(function () {
    var bNow2 = bar();
    if (!bNow2 || bNow2 === boundBar) return;
    boundBar = bNow2;
    if (snapshot) showReview(bNow2, { restored: true, keepResults: true });
    else syncBar();
  }).observe(document.documentElement, { childList: true, subtree: true });
})();
</script>`;

export function renderAttention(
  view: AttentionView,
  coverage: SourceCoverage[],
  provenance: Provenance,
  counts?: Map<string, number | CommentKindCounts>,
  detail?: ProvenanceDetail,
  act?: AttentionAct,
  briefs?: Map<RefKey, ItemBrief>,
  census?: NeedsDrewCensus | null,
): string {
  const sections = attentionSections(view, coverage, counts, act, briefs, census);
  return layout(
    "Needs you",
    `<h1>Needs you</h1><p class="muted">The NEEDS USER queue is an intake the orchestrator works first — it resolves what it can and escalates only what genuinely requires the user, each escalated item with its reason. Everything else the fleet can work is the orchestrator's own queue. Finished runs and stale items expire out (2 h after a run finishes, 24 h at the latest). Machine-first: <a href="/api/attention.json">/api/attention.json</a> · <a href="/api/coverage.json">/api/coverage.json</a>. Incident alerts (watchdog push + ack) live on <a href="/alerts">Alerts</a>.</p>
${sections.header}${sections.queues}${sections.sessions}`,
    provenance,
    "/attention",
    act?.identities ? ACTION_SCRIPT + COMMENT_SCRIPT + BULK_SCRIPT : "",
    detail ? freshnessChips(detail, ["orchestration", "attention", "registry"]) : "",
  );
}

/**
 * The Needs-Drew label census section (#1226): open user/review +
 * user/escalated items across the four repos, straight from GitHub —
 * the labels are the system of record and this view only reads them.
 * census === null means the source cannot census: the section says
 * "unavailable" rather than rendering an empty, confident zero.
 * census === undefined means the caller predates the census (tests,
 * fragments) and the section is omitted entirely.
 */
export function needsDrewCensusHtml(census: NeedsDrewCensus | null | undefined): string {
  if (census === undefined) return "";
  if (census === null) {
    return `<h2 id="needs-drew-census">Needs Drew — GitHub label census</h2><p class="muted">Unavailable — this source cannot census the user/review and user/escalated labels, so this queue cannot be reported here.</p>`;
  }
  const failed = census.sources.filter((x) => !x.ok || x.truncated || x.incomplete);
  const failureNote = failed.length
    ? `<div class="banner">Label census incomplete — ${failed
        .map((x) => {
          if (!x.ok) return `${esc(x.repo)} ${esc(x.label)}: ${esc(x.error ?? "failed")}`;
          // Truncation and GitHub's partial-search signal are different
          // failures: say which one it was instead of blurring them.
          const why = x.incomplete
            ? "search returned incomplete results, count is a floor"
            : "page cap reached, count is a floor";
          return `${esc(x.repo)} ${esc(x.label)}: ${why}`;
        })
        .join("; ")}. The list below may be missing items from those sources.</div>`
    : "";
  const rows = census.items.length
    ? `<ul>${census.items
        .map(
          (i) =>
            `<li><a href="${esc(i.url)}">${esc(i.ref)}</a> ${i.kind === "pr" ? "(PR) " : ""}${esc(i.title)} — ${i.needsLabels
              .map((l) => `<code>${esc(l)}</code>`)
              .join(
                " ",
              )} — ${i.reason ? esc(i.reason) : "no reason recorded on the item — open it for context"} <span class="muted">(updated ${esc(i.updatedAt)})</span></li>`,
        )
        .join("")}</ul>`
    : `<p class="muted">No open items carry user/review or user/escalated right now${census.complete ? "" : " in the sources that answered"}.</p>`;
  return `<h2 id="needs-drew-census">Needs Drew — GitHub label census (${census.items.length})</h2><p class="muted">Every open issue and PR carrying <code>user/review</code> or <code>user/escalated</code> across the four repos (#1226). The labels are the system of record; this view reads them and never writes them. Census taken ${esc(census.fetchedAt)}${census.complete ? "" : " — incomplete, see below"}. Machine form: <a href="/api/needs-drew.json">/api/needs-drew.json</a>.</p>${failureNote}${rows}`;
}

/**
 * The attention page in three independently loadable sections
 * (#1444): header (banner / chips / coverage / standing default),
 * queues (Needs User + Needs Orchestrator + the snoozed & hidden
 * strips), sessions. The full page assembles them; the shell serves
 * them as Datastar fragments from /frag/attention/*.
 */
export function attentionSections(
  view: AttentionView,
  coverage: SourceCoverage[],
  counts?: Map<string, number | CommentKindCounts>,
  act?: AttentionAct,
  briefs?: Map<RefKey, ItemBrief>,
  census?: NeedsDrewCensus | null,
): { header: string; queues: string; sessions: string } {
  const children = sessionChildren(view.all);
  const roots = sessionRoots(view.all);
  // #1139: the tier source's state decides what the counts may claim.
  // Absent/invalid → the orchestrator's published queues are NOT
  // reflected here, so the banner says so and the headings render
  // "unknown" rather than a confident number; stale → the numbers
  // render, badged with the section's age. This banner is also
  // #1148's aggregator-side dead-man's switch: if the orchestration
  // job dies, this is the surface that says so.
  const ts = view.tierSource;
  const tierBanner = !ts.present
    ? ts.state === "invalid"
      ? `<div class="banner">Tier source invalid — the orchestration section is present but failed validation (${esc((ts.failedChecks ?? []).join(", ") || "unknown check")}); showing session-derived items only — the orchestrator's published queues are NOT reflected in the counts below.</div>`
      : `<div class="banner">Tier source missing — showing session-derived items only; the orchestrator's published queues are NOT reflected in the counts below.</div>`
    : ts.state === "stale"
      ? `<div class="banner">Tier source stale — the last orchestration section (${esc(ts.runId ?? "unknown run")}) is ${esc(ts.generatedAt ? feedAgeText(ts.generatedAt) : "old")}; the queues below are its last published state and may have moved since.</div>`
      : "";
  const sectionStamp =
    ts.present && ts.runId
      ? ` <span class="muted">· section ${esc(ts.runId)} · ${esc(ts.generatedAt ? feedAgeText(ts.generatedAt) : "")}</span>`
      : "";
  // #1152: section-derived items disable their controls whenever the
  // tier source is not live — stale items come from the publisher's
  // last section and may no longer exist in its current view (absent/
  // invalid sources serve no section items at all). Session-derived
  // items stay enabled: the session feed is live.
  const sectionDisabledReason =
    ts.state !== "live"
      ? `the tier source is ${ts.state}, so this item cannot be confirmed against the orchestrator's current published queues`
      : undefined;
  const tierAct = (item: TierItem) => {
    const objectRef = tierItemObjectRef(item);
    // #1148 watchdog cards describe the aggregator's CURRENT diagnosis,
    // not the publisher's possibly-obsolete section, so the #1152 gate
    // must not disable them: the orchestration-outage card is exactly
    // the one that has to stay actionable while the tier source is
    // absent/invalid/stale (Codex P2, PR #2431).
    const disabledReason = item.source === "watchdog" ? undefined : sectionDisabledReason;
    return {
      actions: act?.actionsByObject?.get(objectRef),
      identities: act?.identities,
      ...(disabledReason ? { disabledReason } : {}),
      comments: act?.commentsByObject?.get(objectRef),
      commentOpts: act?.commentOpts,
    };
  };
  const sessionAct = act
    ? {
        actionsByObject: act.actionsByObject,
        identities: act.identities,
        commentsByObject: act.commentsByObject,
        commentOpts: act.commentOpts,
      }
    : undefined;

  // #1443/#1449/#1450: partition every queue entry by its render-time
  // effective state. A suppressed entry (active snooze, dismiss, or
  // mark-done — pending counts, exactly like applied) leaves its list
  // NOW and lands in the matching strip with its way back; the counts
  // below are the VISIBLE counts, so acting on a card visibly moves
  // the numbers.
  const now = Date.now();
  const snoozedList: StripEntry[] = [];
  const hiddenList: StripEntry[] = [];
  const inViewRefs = new Set<string>();
  const partition = (
    objectRef: string,
    title: string | null | undefined,
    actions: DashAction[] | undefined,
  ): CardState => {
    inViewRefs.add(objectRef);
    const cs = cardState(actions, now);
    if (cs.suppressed) {
      (cs.suppressed.kind === "snoozed" ? snoozedList : hiddenList).push({ objectRef, title: title ?? undefined, cs });
    }
    return cs;
  };
  interface QueuedCard {
    html: string;
    sinceMs: number;
    seenMs: number;
    rank: number;
  }
  const drewCardsList: QueuedCard[] = [];
  for (const e of view.drew) {
    const objectRef = sessionObjectRef(e.sessionId);
    const cs = partition(objectRef, e.title ?? e.sessionId, act?.actionsByObject?.get(objectRef));
    if (cs.suppressed) continue;
    drewCardsList.push({
      html: bulkPickHtml(objectRef, e.title ?? e.sessionId, act) + sessionCard(e, children, 0, counts, sessionAct),
      sinceMs: Date.parse(e.lastActivityAt ?? "") || 0,
      seenMs: Date.parse(e.lastActivityAt ?? "") || 0,
      rank: 5,
    });
  }
  for (const item of view.drewExtra) {
    const objectRef = tierItemObjectRef(item);
    const cs = partition(objectRef, item.title, act?.actionsByObject?.get(objectRef));
    if (cs.suppressed) continue;
    const drewAct = tierAct(item);
    drewCardsList.push({
      html:
        bulkPickHtml(objectRef, item.title, act, drewAct.disabledReason) +
        tierItemCard(item, "Why you", counts, drewAct) +
        (item.ref ? briefPanelsForRefs([item.ref], briefs) : ""),
      sinceMs: Date.parse(item.since) || 0,
      seenMs: Date.parse(item.lastSeenAt) || 0,
      rank: item.horizon ? (HORIZON_RANK[item.horizon] ?? 5) : 5,
    });
  }
  // #1456: the saved card order applies at render. "urgency" is the
  // published order (no sort); the rest sort the visible list.
  // #1485: every order sorts on the single sinceMs axis (when the card
  // entered the queue), so newest and oldest are exact reverses. seenMs
  // (lastSeenAt) is NOT the axis: a stale card seen recently would lead
  // both orders. Deadline ties break oldest-waiting first (ascending
  // sinceMs), matching the deadline/urgency reading.
  const order = act?.needsOrder ?? "urgency";
  if (order === "newest") drewCardsList.sort((a, b) => b.sinceMs - a.sinceMs);
  else if (order === "oldest") drewCardsList.sort((a, b) => a.sinceMs - b.sinceMs);
  else if (order === "deadline") drewCardsList.sort((a, b) => a.rank - b.rank || a.sinceMs - b.sinceMs);
  const orchCardsList: string[] = [];
  for (const e of view.orchestrator) {
    const objectRef = sessionObjectRef(e.sessionId);
    const cs = partition(objectRef, e.title ?? e.sessionId, act?.actionsByObject?.get(objectRef));
    if (cs.suppressed) continue;
    orchCardsList.push(
      bulkPickHtml(objectRef, e.title ?? e.sessionId, act) + sessionCard(e, children, 0, counts, sessionAct),
    );
  }
  for (const item of view.orchestratorExtra) {
    const objectRef = tierItemObjectRef(item);
    const cs = partition(objectRef, item.title, act?.actionsByObject?.get(objectRef));
    if (cs.suppressed) continue;
    const orchAct = tierAct(item);
    orchCardsList.push(
      bulkPickHtml(objectRef, item.title, act, orchAct.disabledReason) +
        tierItemCard(item, "Why the fleet", counts, orchAct) +
        (item.ref ? briefPanelsForRefs([item.ref], briefs) : ""),
    );
  }
  // Store-only suppressed refs: items the publisher no longer lists
  // (their dismissal was applied and they left the tiers) stay visible
  // — and restorable — here, never silently gone (#1450).
  for (const [objectRef, actions] of act?.actionsByObject?.entries() ?? []) {
    if (inViewRefs.has(objectRef)) continue;
    const cs = cardState(actions, now);
    if (cs.suppressed) {
      (cs.suppressed.kind === "snoozed" ? snoozedList : hiddenList).push({ objectRef, cs });
    }
  }

  const drewVisible = drewCardsList.length;
  const orchVisible = orchCardsList.length;
  const drewCount = ts.present ? String(drewVisible) : "unknown";
  const orchCount = ts.present ? String(orchVisible) : "unknown";
  const chips = `<div class="chips"><a href="#drew">Needs User (${drewCount})</a><a href="#orchestrator">Needs Orchestrator (${orchCount})</a><a href="#sessions">All sessions (${roots.length})</a></div>`;
  // #1290/#1291: the standing delegation-default toggle — a dashboard
  // preference, so it renders whenever the action surface is up; its
  // state comes from the preference records in the store.
  const delegationDefault = act?.identities
    ? delegationDefaultHtml(act.actionsByObject?.get(DELEGATION_DEFAULT_OBJECT_REF), {
        identities: act.identities,
      })
    : "";
  const header = `${tierBanner}${chips}
${coverageHtml(coverage)}
${delegationDefault}`;
  const drewCards =
    drewCardsList.map((c) => c.html).join("") ||
    (ts.present
      ? `<p class="muted">Nothing in the user queue right now.</p>`
      : `<p class="muted">Unknown — the tier source is missing, so this queue cannot be reported.</p>`);
  const orchCards =
    orchCardsList.join("") ||
    (ts.present
      ? `<p class="muted">The fleet's queue is empty.</p>`
      : `<p class="muted">Unknown — the tier source is missing, so this queue cannot be reported.</p>`);
  const snoozedStrip = snoozedList.length
    ? `<h2 id="snoozed">Snoozed by you (${snoozedList.length})</h2><p class="muted">These items are out of the queues until their snooze expires — the orchestration pass omits them, and they resurface on their own. Unsnoozing records a superseding action.</p>${snoozedList.map((e) => stripCardHtml(e, act?.identities)).join("")}`
    : "";
  const hiddenStrip = hiddenList.length
    ? `<details class="hiddenstrip"><summary><strong>Dismissed / done by you (${hiddenList.length})</strong> — hidden from the queues; restore any of them to re-review or add a note</summary>${hiddenList.map((e) => stripCardHtml(e, act?.identities)).join("")}</details>`
    : "";
  const queues = `${bulkBarHtml(act)}${needsOrderHtml(act)}<h2 id="drew">Needs User (${drewCount})${sectionStamp}</h2>${drewCards}
${snoozedStrip}${hiddenStrip}<h2 id="orchestrator">Needs Orchestrator (${orchCount})${sectionStamp}</h2><p class="muted">The orchestration job (#899) reviews this queue and pushes work back into motion; it escalates to the user queue only what truly needs the user.</p>${orchCards}${needsDrewCensusHtml(census)}`;
  const sessionCards = roots.length
    ? roots.map((e) => sessionCard(e, children, 0, counts, sessionAct)).join("")
    : `<p class="muted">No current sessions.</p>`;
  const sessions = `<h2 id="sessions">All sessions (${roots.length})</h2><p class="muted">Every current session with its source and state; sub-sessions collapse under their parent.</p>${sessionCards}
${view.expiredCount ? `<p class="muted">${view.expiredCount} finished or stale ${view.expiredCount === 1 ? "entry" : "entries"} expired out of this view.</p>` : ""}`;
  return { header, queues, sessions };
}

/** Wrap one attention section as a Datastar fragment container: the
 *  shell's placeholder and the fragment payload share this shape, so
 *  a refresh re-patches the container with itself (attrs included). */
export function attentionFragment(section: "header" | "queues" | "sessions", innerHtml: string): string {
  return `<div id="attn-${section}" data-frag data-on-load="@get('/frag/attention/${section}')" data-on-dashrefresh__window="@get('/frag/attention/${section}')">${innerHtml}</div>`;
}

/**
 * The Datastar shell for /attention (#1444): nav + headings paint
 * immediately; the three heavy sections stream in as fragments.
 * Without JS, a meta refresh lands on the full server render.
 */
export function renderAttentionShell(provenance: Provenance, detail?: ProvenanceDetail): string {
  const skel = (label: string) => `<div class="skel">Loading ${label}…</div>`;
  const body = `<h1>Needs you</h1><p class="muted">Loading the Needs User and Needs Orchestrator queues…</p><noscript><meta http-equiv="refresh" content="0; url=/attention?full=1"></noscript>
${attentionFragment("header", skel("queue status"))}
${attentionFragment("queues", skel("the queues"))}
${attentionFragment("sessions", skel("sessions"))}`;
  return layout(
    "Needs you",
    body,
    provenance,
    "/attention",
    ACTION_SCRIPT + COMMENT_SCRIPT + BULK_SCRIPT,
    detail ? freshnessChips(detail, ["orchestration", "attention", "registry"]) : "",
  );
}

// --- Today view (slice 2, #898 §4) -------------------------------------------

function todayHealthStrip(health: HealthView | null, quota: QuotaView, quotaUnavailable: boolean): string {
  const compactAge = (ageMs: number) => ageMsText(ageMs).replace(/ old$/, "");
  const stackServices =
    health?.services.filter(
      (service) => service.def.probe !== "feed" && service.def.probe !== "section" && service.def.probe !== "off",
    ) ?? [];
  const stackUp = stackServices.filter((service) => service.ok === true).length;
  const stackDown = stackServices.filter((service) => service.ok === false);
  const stackUnknown = stackServices.filter((service) => service.ok === null);
  const stackText =
    !health || health.mode !== "live" || stackServices.length === 0
      ? "stack unknown"
      : `stack ${stackUp}/${stackServices.length} up${stackDown.length ? ` · ${stackDown.map((service) => service.def.name).join(", ")} down` : ""}${stackUnknown.length ? ` · unknown: ${stackUnknown.map((service) => service.def.name).join(", ")}` : ""}`;
  const stackState =
    !health || health.mode !== "live" || stackServices.length === 0
      ? "unknown"
      : stackDown.length
        ? "warn"
        : stackUnknown.length
          ? "unknown"
          : "ok";

  const feedAge = health?.feed.ageMs;
  const feedDown = health?.services.some((service) => service.def.id === "fleet-feed" && service.ok === false) ?? false;
  const feedState =
    feedDown || (feedAge !== null && feedAge !== undefined && feedAge > 15 * 60_000)
      ? "warn"
      : feedAge === null || feedAge === undefined
        ? "unknown"
        : "ok";
  const feedText = feedAge === null || feedAge === undefined ? "feed unknown" : `feed ${compactAge(feedAge)}`;

  const orchestration = health?.jobs.orchestration;
  const orchestrationState =
    !orchestration || orchestration.state === "unknown"
      ? "unknown"
      : orchestration.state === "live" && orchestration.ageMs !== undefined
        ? "ok"
        : "warn";
  const orchestrationText =
    !orchestration || orchestration.state === "unknown"
      ? "orchestrator unknown"
      : orchestration.state === "live"
        ? orchestration.ageMs === undefined
          ? "orchestrator age unknown"
          : `orchestrator ${compactAge(orchestration.ageMs)}`
        : `orchestrator ${orchestration.state}`;

  const quotaErrors = quota.gatherer
    ? quota.gatherer.providers.filter((provider) => provider.state === "error").length
    : quota.pools.filter((pool) => pool.state === "error" || pool.freshness?.state === "error").length;
  const quotaState =
    quotaUnavailable || (!quota.gatherer && quota.pools.length === 0)
      ? "unknown"
      : quotaErrors
        ? "warn"
        : quota.gatherer?.ok === false
          ? "unknown"
          : "ok";
  const quotaText =
    quotaUnavailable || (!quota.gatherer && quota.pools.length === 0)
      ? "quota unknown"
      : quotaErrors
        ? `quota errors ${quotaErrors}`
        : quota.gatherer?.ok === false
          ? "quota reads unknown"
          : "quota reads OK";

  const items = [
    { text: stackText, state: stackState, href: "/health", title: "Stack service health" },
    { text: feedText, state: feedState, href: "/health", title: "Fleet feed freshness" },
    { text: orchestrationText, state: orchestrationState, href: "/health", title: "Orchestration freshness" },
    { text: quotaText, state: quotaState, href: "/quota", title: "Quota read status" },
  ];
  return `<div class="today-health-strip" role="group" aria-label="System health">${items.map((item) => `<a class="today-health-link ${item.state}" href="${item.href}" title="${esc(item.title)}">${esc(item.text)}</a>`).join("")}</div>`;
}

/** The today page's body alone (#1444 fragment payload). */
export function renderTodayBody(
  today: TodayView,
  attention: AttentionView,
  quota: QuotaView,
  coverage: SourceCoverage[],
  health: HealthView | null = null,
  quotaUnavailable = false,
  queue?: QueueView,
  queueStale = false,
): string {
  const walled = quota.pools.filter((p) => p.state === "walled");
  // "-direct" display rule (#6125), judged over this view's pool set.
  const poolDisplayNames = providerDisplayNames(quota.pools.map((p) => p.provider));
  const displayPoolProvider = (name: string) => poolDisplayNames.get(name) ?? name;
  const alerts: string[] = [];
  const drewTotal = attention.drew.length + attention.drewExtra.length;
  if (drewTotal) {
    alerts.push(
      `<div class="banner drew"><strong>${drewTotal} item${drewTotal === 1 ? "" : "s"} in the user queue</strong> — <a href="/attention#drew">review them</a></div>`,
    );
  }
  if (walled.length) {
    alerts.push(
      `<div class="banner"><strong>Walled pools:</strong> ${walled.map((p) => esc(`${displayPoolProvider(p.provider)} (${p.plan})`)).join(", ")} — <a href="/quota">quota</a></div>`,
    );
  }
  // #1139: the derived view's banner names the section's TRUE state —
  // absent, invalid, and stale are different failures, and "has not
  // published yet" is only honest for one of them.
  const diag = today.sectionDiagnosis;
  const staleBanner =
    today.fromOrchestrator && diag?.state === "stale"
      ? `<div class="banner">The orchestration section is stale — its last run (${esc(diag.runId ?? "unknown")}) is ${esc(diag.generatedAt ? feedAgeText(diag.generatedAt) : "old")}; the measurements below are its last published state, not a current reading.</div>`
      : "";
  const provenanceLine = today.fromOrchestrator
    ? `<p class="muted">From the orchestration job's goal measurements (section written ${fmtDate(today.generatedAt)}). Machine-first: <a href="/api/today.json">/api/today.json</a> · <a href="/api/orchestration.json">/api/orchestration.json</a>.</p>${staleBanner}`
    : diag?.state === "invalid"
      ? `<div class="banner">Derived from the board, roadmap, and item activity — the orchestration section is present but failed validation (${esc((diag.failedChecks ?? []).join(", ") || "unknown check")}), so this is the aggregator's own derivation, not the orchestrator's measurement.</div>`
      : diag?.state === "stale"
        ? `<div class="banner">Derived from the board, roadmap, and item activity — the last orchestration section (${esc(diag.runId ?? "unknown run")}) is stale (${esc(diag.generatedAt ? feedAgeText(diag.generatedAt) : "old")}) and could not be used, so this is the aggregator's own derivation, not the orchestrator's measurement.</div>`
        : `<div class="banner">Derived from the board, roadmap, and item activity — the orchestration job (#899) has not published a goal-measurement section yet, so this is the aggregator's own derivation, not the orchestrator's measurement.</div>`;
  const actions = today.actions?.length
    ? `<h2>Pushed back into motion</h2>${today.actions.map((a) => `<div class="card"><span class="badge">${esc(a.kind)}</span> <strong>${esc(a.target)}</strong><br>${esc(a.summary)}<br><span class="muted">${esc(fmtDate(a.at))}</span></div>`).join("")}`
    : "";
  // #6174: an objective ref comes from publisher lined-up data and is
  // only sometimes a GitHub ref — tracked-goal items carry their goal id,
  // and refUrl throws on those, which 500ed the whole /today render (the
  // Today-page sibling of the #6149 Needs-you fix). Mirror tierItemCard:
  // GitHub refs link, everything else renders as plain code.
  const objectives = today.objectives.length
    ? today.objectives
        .map(
          (o, i) =>
            `<div class="card obj"><div class="n">${i + 1}</div><div><strong>${esc(o.title)}</strong> <span class="badge src">${esc(o.chip)}</span>${o.detail ? `<br><span class="muted">${esc(o.detail)}</span>` : ""}${o.ref ? `<br>${isValidRef(o.ref) ? `<a href="${esc(refUrl(o.ref))}">${esc(o.ref)}</a>` : `<code>${esc(o.ref)}</code>`}` : ""}</div></div>`,
        )
        .join("")
    : `<p class="muted">No objectives lined up for today.</p>`;
  const momentum = today.momentum.length
    ? today.momentum
        .map((m) => {
          // Defensive: a TodayView built outside derive.ts may still
          // carry a non-array `accomplished` (e.g. a bare string) —
          // coerce instead of crashing the whole /today render.
          const rawAccomplished: unknown = m.accomplished;
          const accomplished = (
            Array.isArray(rawAccomplished) ? rawAccomplished : rawAccomplished ? [rawAccomplished] : []
          ).map((a) => String(a));
          const done = accomplished.length
            ? `<br>Recently accomplished:<ul>${accomplished.map((a) => `<li>${esc(a)}</li>`).join("")}</ul>`
            : "";
          const blocked = m.blockedBy ? `<br><strong>Blocked by:</strong> ${esc(m.blockedBy)}` : "";
          const detail = m.detail ? `<br><span class="muted">${esc(m.detail)}</span>` : "";
          return `<div class="card"><strong>${esc(m.project)}</strong> <span class="badge ${esc(m.state)}">${esc(m.state)}</span>${done}${blocked}${detail}</div>`;
        })
        .join("")
    : `<p class="muted">No projects to measure yet.</p>`;
  const nextUp = queue
    ? `<section class="today-next-up"><h2>Next up <span class="badge">${Math.min(queue.nextUp.length, 5)}</span>${queueStale ? ' <span class="badge stale" title="Queue inputs are stale or unavailable; see /health">stale inputs</span>' : ""}</h2><p class="muted">The next queued work, in the published horizon order.</p>${queueRows(queue.nextUp.slice(0, 5), "Nothing is queued for the next pick.")}</section>`
    : "";
  return `${todayHealthStrip(health, quota, quotaUnavailable)}${provenanceLine}${alerts.join("")}
<h2>Top objectives</h2>${objectives}
<h2>Project momentum</h2>${momentum}
${nextUp}${actions}${coverageHtml(coverage)}`;
}

/** The today page (#1444): full server render. */
export function renderToday(
  today: TodayView,
  attention: AttentionView,
  quota: QuotaView,
  coverage: SourceCoverage[],
  provenance: Provenance,
  detail?: ProvenanceDetail,
  health: HealthView | null = null,
  quotaUnavailable = false,
  queue?: QueueView,
  queueStale = false,
): string {
  return layout(
    "Today",
    `<h1>Today</h1>${renderTodayBody(today, attention, quota, coverage, health, quotaUnavailable, queue, queueStale)}`,
    provenance,
    "/today",
    "",
    detail ? freshnessChips(detail, ["orchestration", "quota", "attention"]) : "",
  );
}

/** The Datastar shell for /today (#1444). */
export function renderTodayShell(
  provenance: Provenance,
  detail: ProvenanceDetail | undefined,
  health: HealthView | null,
  quota: QuotaView,
  quotaUnavailable = false,
): string {
  return layout(
    "Today",
    `<h1>Today</h1><noscript><meta http-equiv="refresh" content="0; url=/today?full=1"></noscript><div id="today-main" data-frag data-on-load="@get('/frag/today')" data-on-dashrefresh__window="@get('/frag/today')">${todayHealthStrip(health, quota, quotaUnavailable)}<div class="skel">Loading today…</div></div>`,
    provenance,
    "/today",
    "",
    detail ? freshnessChips(detail, ["orchestration", "quota", "attention"]) : "",
  );
}

// --- Queue visibility (#1160) -----------------------------------------------

function queueRows(rows: QueueRow[], empty: string): string {
  if (!rows.length) return `<p class="muted">${esc(empty)}</p>`;
  return `<div class="board">${rows
    .map((item) => {
      const area = item.area ? `${item.area} · ${item.repo}` : item.repo;
      const blocked = item.status.toLowerCase().includes("block");
      const age =
        item.ageDays === undefined
          ? blocked
            ? "block age unknown"
            : "age unknown"
          : blocked
            ? `block state last updated ${item.ageDays}d ago`
            : item.ageDays === 0
              ? "updated today"
              : `${item.ageDays}d since update`;
      const blockers = item.blockedBy?.length
        ? `<br><strong>Blocked by:</strong> ${item.blockedBy.map(esc).join("; ")}`
        : "";
      const worker = item.worker ? `<br><strong>Worker:</strong> ${esc(item.worker)}` : "";
      const epic = item.epic
        ? item.epic.url
          ? `<br><span class="muted">Epic: <a href="${esc(item.epic.url)}">${esc(item.epic.title)}</a></span>`
          : `<br><span class="muted">Epic: ${esc(item.epic.title)}</span>`
        : "";
      const why = item.why ? `<br><span class="muted">${esc(item.why)}</span>` : "";
      const horizon = item.horizon ? ` · ${esc(item.horizon)}` : "";
      const title = item.url ? `<a href="${esc(item.url)}">${esc(item.title)}</a>` : esc(item.title);
      return `<article class="card"><span class="badge">${esc(area)}</span> <span class="badge">${esc(item.status)}${horizon}</span><br><strong>${title}</strong>${item.ref ? `<br><span class="muted">${esc(item.ref)}</span>` : ""}<br><span class="muted">${esc(age)} · ${esc(item.source)}</span>${epic}${why}${blockers}${worker}</article>`;
    })
    .join("")}</div>`;
}

/** The queue page body is also the Datastar fragment and JSON's source view. */
export function renderQueueBody(view: QueueView): string {
  return `<p class="muted">Cross-project work, reconciled from the existing board, Project pool, and orchestration inputs. Counts are the rows shown; machine-first: <a href="/api/queue.json">/api/queue.json</a>. Updated ${esc(fmtDate(view.generatedAt))}.</p>
<h2>In progress <span class="badge">${view.inProgress.length}</span></h2>${queueRows(view.inProgress, "Nothing is in progress.")}
<h2>Blocked <span class="badge">${view.blocked.length}</span></h2>${queueRows(view.blocked, "Nothing is blocked.")}
<h2>Done recently <span class="badge">${view.doneRecently.length}</span></h2><p class="muted">Completed or merged in the last 7 days; retired bookkeeping cards are excluded.</p>${queueRows(view.doneRecently, "Nothing completed in the last 7 days.")}
<h2>Next up <span class="badge">${view.nextUp.length}</span></h2><p class="muted">Queued work ordered by the published horizon ladder.</p>${queueRows(view.nextUp, "Nothing is queued for the next pick.")}`;
}

export function renderQueue(view: QueueView, provenance: Provenance, detail?: ProvenanceDetail): string {
  return layout(
    "Queue",
    `<h1>Queue visibility</h1>${renderQueueBody(view)}`,
    provenance,
    "/queue",
    "",
    detail ? freshnessChips(detail, ["board", "orchestration", "items"]) : "",
  );
}

export function renderQueueShell(provenance: Provenance, detail?: ProvenanceDetail): string {
  return layout(
    "Queue",
    `<h1>Queue visibility</h1><noscript><meta http-equiv="refresh" content="0; url=/queue?full=1"></noscript><div id="queue-main" data-frag data-on-load="@get('/frag/queue')" data-on-dashrefresh__window="@get('/frag/queue')"><div class="skel">Loading queue…</div></div>`,
    provenance,
    "/queue",
    "",
    detail ? freshnessChips(detail, ["board", "orchestration", "items"]) : "",
  );
}

// --- Overview / front door (#1134) -------------------------------------------
// "/" is the stack front door: one glance at what needs Drew, what is
// moving, what is broken, and where the quotas sit. Every number here
// is derived from the same source views the dedicated pages render
// (attention, the Project pool, quota, health, the board) — the
// overview owns no data of its own, so its counts cannot drift from
// their source pages. deriveOverview is pure, so the HTML page and
// /api/overview.json serve the identical summary.

export interface OverviewData {
  attention: AttentionView;
  pool: ProjectPoolItem[];
  quota: QuotaView;
  health: HealthView;
  board: BoardItem[];
  /** Resolved board refs, used to distinguish open linked PRs. */
  items?: Map<RefKey, GhItem>;
  /** The same line-up source used by /projects for disagreement badges. */
  orchestration?: OrchestrationSection | null;
  /** Dashboard actions by object ref, so the needs-you summary applies
   *  the same render-time suppression as /attention (Codex review on
   *  PR 2424). Absent in tests and fixture renders: nothing suppressed. */
  actionsByObject?: Map<string, DashAction[]>;
}

export type OverviewInProgressSignal = "project-status" | "active-session" | "open-pr";

export interface OverviewInProgressItem {
  ref: RefKey;
  title: string;
  repo: string;
  url: string;
  signals: OverviewInProgressSignal[];
  disagreement: { project: string; derived: string } | null;
}

export interface OverviewSummary {
  /** count is null when the tier source is not present: the queue is
   *  unknown, never a confident zero (Codex review on PR 2424). */
  needsYou: { count: number | null; tierState: string; top: { title: string; detail?: string }[] };
  projects: { repo: string; total: number; inProgress: number; inReview: number; done: number }[];
  quota: { provider: string; plan: string; state: QuotaState; worstPercent: number | null }[];
  health: { degraded: boolean; up: number; down: number; total: number; feedAgeMs: number | null };
  recentDone: { title: string; repo: string }[];
  inProgress: OverviewInProgressItem[];
}

function reconciledInProgress(data: OverviewData): OverviewInProgressItem[] {
  const poolByRef = new Map(data.pool.map((item) => [item.ref, item]));
  const rows = new Map<
    RefKey,
    {
      ref: RefKey;
      title: string;
      repo: string;
      url: string;
      signals: Set<OverviewInProgressSignal>;
    }
  >();
  const ensure = (ref: RefKey, title?: string, repo?: string, url?: string) => {
    const project = poolByRef.get(ref);
    const item = data.items?.get(ref);
    const existing = rows.get(ref);
    if (existing) return existing;
    const hashAt = ref.lastIndexOf("#");
    const row = {
      ref,
      title: project?.title ?? item?.title ?? title ?? ref,
      repo: project?.repo ?? (hashAt >= 0 ? ref.slice(0, hashAt) : (repo ?? "")),
      url: project?.issueUrl ?? item?.url ?? url ?? refUrl(ref),
      signals: new Set<OverviewInProgressSignal>(),
    };
    rows.set(ref, row);
    return row;
  };
  const signal = (ref: RefKey, kind: OverviewInProgressSignal, title?: string, repo?: string, url?: string) => {
    ensure(ref, title, repo, url).signals.add(kind);
  };

  for (const item of data.pool) {
    if (item.status === "In Progress") signal(item.ref, "project-status", item.title, item.repo, item.issueUrl);
    if (item.contentKind === "pr" && item.state === "open")
      signal(item.ref, "open-pr", item.title, item.repo, item.issueUrl);
  }
  for (const goal of data.orchestration?.goals ?? []) {
    for (const task of goal.linedUp) {
      if (task.status !== "in-progress") continue;
      if (task.ref) {
        // #6205: a lined-up ref is feed data, not necessarily a GitHub
        // ref (goal ids, bare numbers). Signaling an invalid ref makes
        // ensure() fall back to refUrl(), whose parseRef throws and
        // kills the whole overview — skip refs the shared rule rejects.
        if (isValidRef(task.ref)) signal(task.ref, "active-session", task.title);
        continue;
      }
      for (const ref of refsInText(task.title)) signal(ref, "active-session", task.title);
      for (const match of task.title.matchAll(/([\w.-]+\/[\w.-]+)#(\d+)/g)) {
        const ref = `${match[1]}#${match[2]}`;
        // #6205: same guard for title-scraped refs — \d+ admits
        // numbers parseRef rejects as unsafe integers.
        if (isValidRef(ref)) signal(ref, "active-session", task.title);
      }
    }
  }
  for (const boardItem of data.board) {
    const openPr = boardItem.links.some((link) => {
      const item = data.items?.get(link.ref);
      return item?.kind === "pr" && item.state === "open";
    });
    for (const link of boardItem.links) {
      const item = data.items?.get(link.ref);
      const title = item?.title ?? boardItem.title;
      const repo = item?.ref.slice(0, link.ref.lastIndexOf("#")) ?? boardItem.repo;
      if (boardItem.source === "orchestrator" && boardItem.orchStatus === "in-progress") {
        signal(link.ref, "active-session", title, repo, item?.url);
      }
      // A board card ties its linked issue and PR to the same work item.
      if (openPr) signal(link.ref, "open-pr", title, repo, item?.url);
    }
  }

  const additionalInProgressRefs = new Set<RefKey>();
  for (const row of rows.values()) {
    if (row.signals.has("active-session") || row.signals.has("open-pr")) additionalInProgressRefs.add(row.ref);
  }
  const signalOrder: OverviewInProgressSignal[] = ["project-status", "active-session", "open-pr"];
  return [...rows.values()]
    .map((row) => {
      const project = poolByRef.get(row.ref);
      const disagreement = project ? projectDisagreement(project, data.orchestration, additionalInProgressRefs) : null;
      return {
        ref: row.ref,
        title: row.title,
        repo: row.repo,
        url: row.url,
        signals: signalOrder.filter((item) => row.signals.has(item)),
        disagreement,
      };
    })
    .sort((a, b) => a.repo.localeCompare(b.repo) || a.title.localeCompare(b.title) || a.ref.localeCompare(b.ref));
}

export function deriveOverview(data: OverviewData): OverviewSummary {
  // The needs-you summary must agree with /attention on the same data:
  // suppressed entries (snoozed / dismissed / marked done — pending
  // counts, exactly as on the attention page) leave the count and the
  // top list, and when the tier source is not present the count is
  // unknown rather than an authoritative number over a partial queue.
  const now = Date.now();
  const suppressed = (objectRef: string): boolean =>
    cardState(data.actionsByObject?.get(objectRef), now).suppressed !== null;
  const drewVisible = data.attention.drew.filter((e) => !suppressed(sessionObjectRef(e.sessionId)));
  const drewExtraVisible = data.attention.drewExtra.filter((item) => !suppressed(tierItemObjectRef(item)));
  const tierPresent = data.attention.tierSource.present;
  const top: { title: string; detail?: string }[] = tierPresent
    ? [
        ...drewExtraVisible.map((t) => ({ title: t.title, detail: t.reason })),
        ...drewVisible.map((e) => ({ title: e.title ?? e.sessionId, detail: e.drewReason })),
      ].slice(0, 5)
    : [];
  const byRepo = new Map<string, { repo: string; total: number; inProgress: number; inReview: number; done: number }>();
  for (const item of data.pool) {
    const row = byRepo.get(item.repo) ?? { repo: item.repo, total: 0, inProgress: 0, inReview: 0, done: 0 };
    row.total += 1;
    if (item.status === "In Progress") row.inProgress += 1;
    else if (item.status === "In Review") row.inReview += 1;
    else if (item.status === "Done") row.done += 1;
    byRepo.set(item.repo, row);
  }
  // "-direct" display rule (#6125), judged over this view's pool set.
  const quotaDisplayNames = providerDisplayNames(data.quota.pools.map((pool) => pool.provider));
  const quota = data.quota.pools.map((pool) => {
    // #1134 review (Codex P2): derive from used/limit with the shared
    // helper — a window that carries `used` and `limit` but no explicit
    // `percent` must not silently drop out of the tightest-window figure.
    const percents = pool.windows.map((w) => windowPercent(w));
    return {
      provider: quotaDisplayNames.get(pool.provider) ?? pool.provider,
      plan: pool.plan,
      state: pool.state,
      worstPercent: percents.length ? Math.max(...percents) : null,
    };
  });
  const services = data.health.services;
  const byRecency = (a: BoardItem, b: BoardItem) => (b.updatedAt ?? "").localeCompare(a.updatedAt ?? "");
  return {
    needsYou: {
      count: tierPresent ? drewVisible.length + drewExtraVisible.length : null,
      tierState: data.attention.tierSource.state,
      top,
    },
    projects: [...byRepo.values()].sort((a, b) => b.total - a.total || a.repo.localeCompare(b.repo)),
    quota,
    health: {
      degraded: data.health.degraded,
      up: services.filter((svc) => svc.ok === true).length,
      down: services.filter((svc) => svc.ok === false).length,
      total: services.length,
      feedAgeMs: data.health.feed.ageMs,
    },
    recentDone: data.board
      .filter((b) => b.status === "done")
      .sort(byRecency)
      .slice(0, 5)
      .map((b) => ({ title: b.title, repo: b.repo })),
    inProgress: reconciledInProgress(data),
  };
}

export function renderOverviewBody(summary: OverviewSummary): string {
  // #1134 review (Codex P2): the stale note is about the tier source, not
  // about the count, so an empty last published queue must still say it.
  const staleNote = `<p class="muted">Tier source stale — this is its last published state and may have moved since.</p>`;
  const needs =
    summary.needsYou.count === null
      ? `<div class="banner">Needs-you count <strong>unknown</strong> — the tier source is ${esc(summary.needsYou.tierState)}, so the orchestrator's published queues are not reflected here. <a href="/attention">Needs you</a></div>`
      : summary.needsYou.count
        ? `<div class="banner drew"><strong>${summary.needsYou.count} item${summary.needsYou.count === 1 ? "" : "s"} need you</strong> — <a href="/attention">review them</a></div>${summary.needsYou.tierState === "stale" ? staleNote : ""}`
        : `<p class="muted">Nothing needs you right now.</p>${summary.needsYou.tierState === "stale" ? staleNote : ""}`;
  const topItems = summary.needsYou.top.length
    ? `<ul>${summary.needsYou.top.map((t) => `<li><strong>${esc(t.title)}</strong>${t.detail ? ` — ${esc(t.detail)}` : ""}</li>`).join("")}</ul>`
    : "";
  const projectCards = summary.projects.length
    ? summary.projects
        .map(
          (proj) =>
            `<div class="card"><strong>${esc(proj.repo)}</strong> <span class="muted">${proj.total} tracked</span><br>${proj.inProgress} in progress · ${proj.inReview} in review · ${proj.done} done — <a href="/program#projects-pool">projects</a></div>`,
        )
        .join("")
    : `<p class="muted">No Project pool items collected yet.</p>`;
  const quotaStrip = summary.quota.length
    ? summary.quota
        .map((pool) => {
          const pct = pool.worstPercent !== null ? ` · ${Math.round(pool.worstPercent)}% of tightest window` : "";
          return `<div class="card"><strong>${esc(pool.provider)}</strong> <span class="muted">${esc(pool.plan)}</span> <span class="badge ${esc(pool.state)}">${esc(pool.state)}</span>${pct}</div>`;
        })
        .join("")
    : `<p class="muted">No quota pools measured yet.</p>`;
  const healthStrip = `<div class="card"><span class="dot ${summary.health.down || summary.health.degraded ? "down" : "up"}"></span><strong>${summary.health.up} of ${summary.health.total} services up</strong>${summary.health.down ? ` · ${summary.health.down} down` : ""}${summary.health.degraded ? ` · <span class="badge stale">degraded</span>` : ""}${summary.health.feedAgeMs !== null ? ` · feed ${esc(ageMsText(summary.health.feedAgeMs))} old` : ""} — <a href="/health">health</a></div>`;
  const itemList = (items: { title: string; repo: string }[], empty: string) =>
    items.length
      ? `<ul>${items.map((i) => `<li>${esc(i.title)}${i.repo ? ` <span class="muted">${esc(i.repo)}</span>` : ""}</li>`).join("")}</ul>`
      : `<p class="muted">${empty}</p>`;
  const inProgressItems = summary.inProgress.length
    ? `<ul>${summary.inProgress
        .map((item) => {
          const signals = item.signals
            .map(
              (signal) =>
                `<span class="badge ${signal === "open-pr" ? "open" : signal === "active-session" ? "orch" : "src"}">${signal === "project-status" ? "Project Status" : signal === "active-session" ? "Active session" : "Open PR"}</span>`,
            )
            .join(" ");
          const clash = item.disagreement
            ? ` <span class="badge stale" title="The Project Status and the state the rest of the dashboard derives disagree — both are shown, neither is preferred">⚠ project: ${esc(item.disagreement.project)} · derived: ${esc(item.disagreement.derived)}</span>`
            : "";
          return `<li><a href="${esc(item.url)}">${esc(item.title)}</a> <span class="muted">${esc(item.repo)}</span> ${signals}${clash}</li>`;
        })
        .join("")}</ul>`
    : `<p class="muted">Nothing in progress from the Project, active-session, or open-PR signals.</p>`;
  return `<p class="muted">One glance at the whole stack: what needs you, what is moving, what is broken, and where the quotas sit. Every number is the same one its source page shows. Machine-first: <a href="/api/overview.json">/api/overview.json</a>.</p>
<h2>Needs you (${summary.needsYou.count ?? "unknown"})</h2>${needs}${topItems}
<h2>Projects</h2>${projectCards}
<h2>Quota</h2>${quotaStrip}
<h2>Health</h2>${healthStrip}
<h2>In progress</h2>${inProgressItems}
<h2>Recently done</h2>${itemList(summary.recentDone, "Nothing done on the board yet.")}
<p class="muted">In progress reconciles Project Status, active sessions lined up, and open PRs; recently done is the board's lane — the full curated cross-repo board is a drill-down from here: <a href="/board">Board →</a></p>`;
}

/** The overview page (#1134): full server render. */
export function renderOverview(data: OverviewData, provenance: Provenance, detail?: ProvenanceDetail): string {
  return layout(
    "Overview",
    `<h1>Overview</h1>${renderOverviewBody(deriveOverview(data))}`,
    provenance,
    "/",
    "",
    detail ? freshnessChips(detail, ["board", "quota", "attention"]) : "",
  );
}

/** The Datastar shell for / (#1134, on the #1444 pattern). */
export function renderOverviewShell(provenance: Provenance, detail?: ProvenanceDetail): string {
  return layout(
    "Overview",
    `<h1>Overview</h1><noscript><meta http-equiv="refresh" content="0; url=/?full=1"></noscript><div id="overview-main" data-frag data-on-load="@get('/frag/overview')" data-on-dashrefresh__window="@get('/frag/overview')"><div class="skel">Loading the overview…</div></div>`,
    provenance,
    "/",
    "",
    detail ? freshnessChips(detail, ["board", "quota", "attention"]) : "",
  );
}

// --- DNS register page (#906 §1, data from #902) -----------------------------

function dnsTable(entries: DnsEntry[], renewingSoon: Set<DnsEntry>, nextRenewal?: DnsEntry | null): string {
  const rows = entries
    .map(
      (e) =>
        `<tr${e === nextRenewal ? ' id="dns-next-renewal"' : ""} data-dns-row data-dns-status="${esc(e.status)}" data-dns-renewing-soon="${renewingSoon.has(e)}"><td><strong id="${esc(dnsAnchor(e.domain))}">${esc(e.domain)}</strong></td><td>${esc(e.renewal)}</td><td>${e.priceUsd != null ? `$${e.priceUsd.toFixed(2)}` : "—"}</td><td>${esc(e.autoRenew)}</td><td>${esc(e.plan)}</td><td>${esc(e.review)}</td></tr>`,
    )
    .join("");
  return `<div class="tblwrap"><table id="dns-register-table"><caption>Domain register</caption><thead><tr><th scope="col">Domain</th><th scope="col">Next renewal</th><th scope="col">Price (list)</th><th scope="col">Auto-renew</th><th scope="col">Plan / status</th><th scope="col">Review?</th></tr></thead><tbody>${rows || '<tr><td colspan="6" class="muted">none</td></tr>'}</tbody></table></div>`;
}

const DNS_FILTER_SCRIPT = `<script>
(function () {
  var buttons = document.querySelectorAll("[data-dns-filter]");
  var rows = document.querySelectorAll("[data-dns-row]");
  buttons.forEach(function (button) {
    button.addEventListener("click", function () {
      var filter = button.getAttribute("data-dns-filter");
      buttons.forEach(function (candidate) {
        var selected = candidate === button;
        candidate.classList.toggle("primary", selected);
        candidate.setAttribute("aria-pressed", String(selected));
      });
      rows.forEach(function (row) {
        var matches = filter === "all" || (filter === "renewing-soon"
          ? row.getAttribute("data-dns-renewing-soon") === "true"
          : row.getAttribute("data-dns-status") === filter);
        row.hidden = !matches;
      });
    });
  });
  var nextLink = document.querySelector("[data-dns-next-link]");
  if (nextLink) {
    nextLink.addEventListener("click", function () {
      buttons.forEach(function (candidate) {
        var selected = candidate.getAttribute("data-dns-filter") === "all";
        candidate.classList.toggle("primary", selected);
        candidate.setAttribute("aria-pressed", String(selected));
      });
      rows.forEach(function (row) { row.hidden = false; });
    });
  }
})();
</script>`;

export function renderDns(section: DnsSection | null, provenance: Provenance, detail?: ProvenanceDetail): string {
  const chips = detail ? freshnessChips(detail, ["dns"]) : "";
  if (!section) {
    return layout(
      "DNS register",
      `<h1>DNS / domain register</h1><div class="banner">The register has not reached the dashboard yet — the fleet-feed collector embeds it from #902's register file, and no section is in the current feed. The register of record is the file the #902 pipeline maintains; this page only renders it.</div>`,
      provenance,
      "/dns",
      "",
      chips,
    );
  }
  const s = summarizeDns(section);
  const next = s.nextRenewal;
  const soonTotal = Math.round(s.renewingSoon.reduce((sum, e) => sum + (e.priceUsd ?? 0), 0) * 100) / 100;
  const renewingSoon = new Set(s.renewingSoon);
  const filters = [
    ["all", "All", section.entries.length],
    ["active", "Active", s.active.length],
    ["earmarked", "Earmarked", s.earmarked.length],
    ["unused", "Unused", s.unused.length],
    ["renewing-soon", "Renewing soon", s.renewingSoon.length],
  ] as const;
  const filterChips = filters
    .map(
      ([filter, label, count], index) =>
        `<button type="button" class="qchip dns-filter${index === 0 ? " primary" : ""}" data-dns-filter="${filter}" aria-pressed="${index === 0}">${label} (${count})</button>`,
    )
    .join("");
  const body = `<h1>DNS / domain register</h1>
<p class="muted">${section.entries.length} domains · $${section.totalAnnualListUsd.toFixed(2)}/yr at list renewal prices · register parsed ${fmtDate(section.generatedAt)} (${esc(section.source)}). The register is maintained by the #902 pipeline — this page renders it, never edits it. Machine-first: <a href="/api/dns.json">/api/dns.json</a>.</p>
<div class="board">
<div class="card"><strong>Next renewal</strong><br>${
    next
      ? `${esc(next.renewal)}${next.priceUsd != null ? ` · $${next.priceUsd.toFixed(2)}` : ""}<br><span class="muted">See <a href="#dns-next-renewal" data-dns-next-link>its row in the renewal-sorted register</a>.</span>`
      : "—"
  }</div>
<div class="card"><strong>Renewing in 45 days</strong><br>${s.renewingSoon.length} domain${s.renewingSoon.length === 1 ? "" : "s"} · $${soonTotal.toFixed(2)} at list price</div>
<div class="card"><strong>Auto-renew</strong><br>${s.autoRenew.on} ON · ${s.autoRenew.off} OFF · ${s.autoRenew.unknown} unknown</div>
<div class="card"><strong>Portfolio shape</strong><br>${s.active.length} in use · ${s.earmarked.length} unused but earmarked · ${s.unused.length} completely unused</div>
</div>
<h2>Domain register (${section.entries.length})</h2>
<div class="chips" role="group" aria-label="Filter domains by status">${filterChips}</div>
${dnsTable(section.entries, renewingSoon, next)}`;
  return layout("DNS register", body, provenance, "/dns", DNS_FILTER_SCRIPT, chips);
}

// --- AA cost explorer page (ainventory import data via the fleet feed) -----

/**
 * The Cost Explorer's client side: tabs, the shared filter bar, the
 * three frontier charts, and both tables, all rendered from the
 * #xpl-data JSON the server embeds (aaPoints + catalog, computed in
 * src/aa.ts). Dependency-free inline script, like COMMENT_SCRIPT.
 * Written without template literals on purpose — it lives inside one.
 */
const AA_EXPLORER_SCRIPT = `<script type="module">
import { applyInequalityFilters, bandIndex, computeRecommendations, logEdges, normName, paretoSet, pctRank, projectTask, scoreEdges } from "/assets/aa-explorer-computations.js";
(function () {
  var dataEl = document.getElementById("xpl-data");
  if (!dataEl) return;
  var DATA = JSON.parse(dataEl.textContent);
  var AA_POINTS = DATA.points || [];
  var CAI_POINTS = DATA.caiPoints || [];
  var CATALOG = DATA.catalog || null;
  var POINTS = AA_POINTS;
  // Default benchmark vintage (2026-11-01, Drew): the payload carries
  // every vintage the AA import holds (v5 beside v4.3.2 today); ranking
  // across vintages compares scores from different methodologies, so
  // the first-load view pins the newest vintage present. "All
  // benchmarks" (bench "") stays one click away in the filter, and
  // saved setups / perspective switches keep their existing semantics.
  function benchVersionKey(b) {
    var m = /v(\d+(?:\.\d+)*)/.exec(b || "");
    return m ? m[1].split(".").map(function (n) { return parseInt(n, 10) || 0; }) : null;
  }
  function cmpVer(a, b) {
    var n = Math.max(a.length, b.length);
    for (var i = 0; i < n; i++) { var d = (a[i] || 0) - (b[i] || 0); if (d) return d; }
    return 0;
  }
  function latestBench(points) {
    var best = "", bestKey = null;
    (points || []).forEach(function (p) {
      var k = benchVersionKey(p.benchmark);
      if (!k) return;
      if (!bestKey || cmpVer(k, bestKey) > 0) { bestKey = k; best = p.benchmark; }
    });
    return best;
  }
  var PALETTE = ["#58a6ff","#3fb950","#e3b341","#f778ba","#bc8cff","#ffa657","#79c0ff","#56d364","#ff7b72","#a5d6ff","#7ee787","#f0883e","#d2a8ff","#e67e22","#2166ac","#b2182b","#1a9850","#386cb0","#7570b3","#66a61e","#e7298a","#bf812d","#0891b2","#ec4899","#2f9e6e","#d97706","#3b82f6","#a855f7","#94a3b8"];
  var famColor = {};
  var famList = [];
  // Colors are assigned over the union of both benchmark families so a
  // family keeps its color when the perspective switches.
  AA_POINTS.concat(CAI_POINTS).forEach(function (p) {
    if (!(p.family in famColor)) { famColor[p.family] = PALETTE[famList.length % PALETTE.length]; famList.push(p.family); }
  });
  var EORD = { "non-reasoning": 0, "low": 1, "medium": 2, "high": 3, "xhigh": 4, "max": 5, "reasoning": 6 };
  function eord(e) { var v = EORD[String(e || "").toLowerCase()]; return v == null ? 50 : v; }
  var ESHORT = { "medium": "med", "reasoning": "reas", "max (peak)": "max·pk", "max (off-peak, derived)": "max·offpk", "non-reasoning": "base" };
  function shortLabel(p) { var e = String(p.effort || ""); return p.family + " " + (ESHORT[e.toLowerCase()] || e); }
  var goCapByName = {};
  (CATALOG || []).forEach(function (e) { if (e.goCapUsd != null) goCapByName[normName(e.name)] = e.goCapUsd; });
  function goCapFor(p) { var v = goCapByName[normName(p.family)]; return v == null ? null : v; }
  function fmtInt(v) { return String(Math.round(v)).replace(/\B(?=(\d{3})+(?!\d))/g, ","); }
  function tasksAt(cost, budget) { return cost ? Math.floor(budget / cost) : null; }
  function esc(s) {
    return String(s == null ? "" : s).replace(/[&<>"']/g, function (c) {
      return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c];
    });
  }
  function fmt$(v) { return v == null ? "—" : "$" + (v < 0.01 ? v.toFixed(4) : v < 1 ? v.toFixed(3) : v.toFixed(2)); }
  function fmtT(v) {
    if (v == null) return "—";
    if (v < 60) return (Math.round(v * 10) / 10) + "s";
    return Math.floor(v / 60) + "m " + Math.round(v % 60) + "s";
  }
  function fmtN(v, d) { return v == null ? "—" : String(Math.round(v * Math.pow(10, d)) / Math.pow(10, d)); }
  function fmtCtx(v) {
    if (v == null) return "—";
    if (v >= 1000000) return (Math.round(v / 100000) / 10) + "M";
    if (v >= 1000) return Math.round(v / 1000) + "k";
    return String(v);
  }
  function cmpDesc(a, b) {
    if (a == null && b == null) return 0;
    if (a == null) return 1;
    if (b == null) return -1;
    return b - a;
  }
  function cmpAsc(a, b) {
    if (a == null && b == null) return 0;
    if (a == null) return 1;
    if (b == null) return -1;
    return a - b;
  }
  // --- benchmark perspective helpers -------------------------------------
  // The two benchmark families (AA Intelligence Index / Coding Agent
  // Index) are never converted into each other; the perspective only
  // swaps which point set is active and what its score is called.
  function scoreWord() { return S.persp === "cai" ? "CAI score" : "Intelligence"; }
  function scoreAxis() { return S.persp === "cai" ? "Coding Agent Index" : "Intelligence Index"; }
  function timeWord() { return S.persp === "cai" ? "agent wall time" : "time"; }
  var S = {
    tab: location.hash === "#models" ? "models" : "aa",
    persp: "aa",
    xp: "api",
    q: "",
    creator: "",
    fam: "",
    bench: latestBench(AA_POINTS),
    efforts: {},
    effortAny: false,
    off: {},
    elig: false,
    pooledOnly: false,
    measuredOnly: false,
    hideDisabled: false,
    sel: null,
    hidden: [],
    sizeBy: {},
    tasksMode: "per",
    baseline: "",
    pfCut: "50",
    bandCount: 3,
    bandScale: 1,
    rec: { allow: 5, speedPriority: 85, priceAllow: 500, slowAllow: 2, altWindow: 2, diversity: "lab", dominance: "lab", perBand: 3, maxCost: "", maxSec: "" },
    pj: { turns: 3, inputTokens: 20000, answerTokens: 2000, thinkingTokens: 4000, cacheReadPct: 50, writesPct: 0, overheadSec: 10, window: "peak" },
    ineq: { mode: "off", combine: "and", conds: [{ metric: "score", op: ">=", value: "" }, { metric: "cost", op: "<=", value: "" }, { metric: "time", op: "<=", value: "" }] },
  };
  var XP_NOTES = {
    plan: "Plan perspective — plan quotas are sunk cost, so dollars are ignored: ranked on intelligence per second (speed + intelligence). Cost columns hidden.",
    api: "API perspective — ranked on intelligence per dollar and cost per task; the cost × score frontier is the value curve.",
    pertask: "Per-task perspective — ranked on total time and total cost to finish one task (the import's measured per-task figures, TTFT shown where timed).",
  };
  function basePoints() {
    return POINTS.filter(function (p) {
      if (S.q && (p.family + " " + (p.effort || "") + " " + p.creator).toLowerCase().indexOf(S.q) < 0) return false;
      if (S.creator && p.creator !== S.creator) return false;
      if (S.fam && p.family !== S.fam) return false;
      if (S.bench && p.benchmark !== S.bench) return false;
      if (S.effortAny && !S.efforts[p.effort || ""]) return false;
      if (S.elig && !p.eligible) return false;
      if (S.off[p.family]) return false;
      if (S.hidden.indexOf(p.key) >= 0) return false;
      return true;
    });
  }
  // --- inequality filters with Pareto-gap metrics (Site 1) ---------------
  // Gaps are measured against the observed frontier of the current scope
  // (all other filters applied): score gap = best score at or below this
  // cost minus this score; price premium = this cost over the cheapest
  // cost at or above this score; speed gap = score gap against time.
  function visPoints() {
    var base = basePoints();
    return applyInequalityFilters(base, S.ineq.mode, S.ineq.combine, S.ineq.conds);
  }
  function visCatalog() {
    if (!CATALOG) return [];
    return CATALOG.filter(function (e) {
      if (S.q && (e.id + " " + e.name + " " + e.providerName + " " + (e.plan || "")).toLowerCase().indexOf(S.q) < 0) return false;
      if (S.fam && e.provider !== S.fam) return false;
      if (S.pooledOnly && !e.routes.length) return false;
      if (S.measuredOnly && e.aaScore == null) return false;
      if (S.hideDisabled && e.disabled) return false;
      return true;
    });
  }
  function sortPoints(rows) {
    var out = rows.slice();
    if (S.xp === "plan") {
      out.sort(function (a, b) { return cmpDesc(a.scorePerSecond, b.scorePerSecond) || cmpDesc(a.score, b.score); });
    } else if (S.xp === "pertask") {
      out.sort(function (a, b) { return cmpAsc(a.time, b.time) || cmpAsc(a.cost, b.cost); });
    } else {
      out.sort(function (a, b) {
        var ka = a.pointsPerDollar != null ? a.pointsPerDollar : a.scorePerDollar;
        var kb = b.pointsPerDollar != null ? b.pointsPerDollar : b.scorePerDollar;
        return cmpDesc(ka, kb) || cmpDesc(a.score, b.score);
      });
    }
    return out;
  }
  function effCost(e) {
    return e.aaCostPerTask != null && e.multiplier ? e.aaCostPerTask / e.multiplier : null;
  }
  function sortCatalog(rows) {
    var out = rows.slice();
    if (S.xp === "plan") {
      out.sort(function (a, b) {
        var ka = a.aaScore != null && a.aaTimePerTask ? a.aaScore / a.aaTimePerTask : null;
        var kb = b.aaScore != null && b.aaTimePerTask ? b.aaScore / b.aaTimePerTask : null;
        return cmpDesc(ka, kb) || cmpDesc(a.aaScore, b.aaScore) || (a.id < b.id ? -1 : 1);
      });
    } else if (S.xp === "pertask") {
      out.sort(function (a, b) { return cmpAsc(a.aaTimePerTask, b.aaTimePerTask) || cmpDesc(a.aaScore, b.aaScore); });
    } else {
      out.sort(function (a, b) { return cmpAsc(effCost(a), effCost(b)) || cmpDesc(a.aaScore, b.aaScore) || (a.id < b.id ? -1 : 1); });
    }
    return out;
  }
  // --- charts (hand-rolled SVG, the reference design's three scatters) ---
  var W = 680, H = 400, ML = 46, MR = 14, MT = 12, MB = 40, PW = W - ML - MR, PH = H - MT - MB;
  function logPos(v, lo, hi) { return (Math.log10(Math.max(v, lo)) - Math.log10(lo)) / (Math.log10(hi) - Math.log10(lo)); }
  function fmtTickX(v, kind) {
    if (kind === "cost") return "$" + (v < 1 ? v : Math.round(v));
    return v < 60 ? v + "s" : v / 60 + "m";
  }
  function drawChart(id, cfg) {
    var svg = document.getElementById(id);
    if (!svg) return;
    var all = visPoints();
    var rows = all.filter(function (p) { return cfg.x(p) != null && cfg.y(p) != null; });
    var note = svg.parentElement.querySelector("[data-note]");
    if (note) {
      var missing = all.filter(function (p) { return cfg.x(p) == null || cfg.y(p) == null; });
      var names = missing.slice(0, 4).map(shortLabel).join(", ");
      note.textContent = rows.length + " of " + all.length + " shown — the rest lack a measured " + cfg.need + " in the import (nothing is estimated)" +
        (missing.length ? ": " + names + (missing.length > 4 ? " (+" + (missing.length - 4) + " more)" : "") + "." : ".");
    }
    if (!rows.length) { svg.innerHTML = ""; return; }
    var xs = rows.map(cfg.x), ys = rows.map(cfg.y);
    var xlo, xhi, ylo, yhi;
    if (cfg.xlog) {
      xlo = Math.pow(10, Math.floor(Math.log10(Math.min.apply(null, xs)) * 2) / 2) / 1.4;
      xhi = Math.max.apply(null, xs) * 1.5;
    } else {
      xlo = Math.min.apply(null, xs); xhi = Math.max.apply(null, xs);
    }
    if (cfg.ylog) {
      ylo = Math.pow(10, Math.floor(Math.log10(Math.min.apply(null, ys)) * 2) / 2) / 1.4;
      yhi = Math.max.apply(null, ys) * 1.5;
    } else {
      ylo = Math.max(0, Math.floor(Math.min.apply(null, ys) - 4));
      yhi = Math.ceil(Math.max.apply(null, ys) + 3);
    }
    var X = function (v) { return ML + (cfg.xlog ? logPos(v, xlo, xhi) : (v - xlo) / (xhi - xlo)) * PW; };
    var Y = function (v) { return MT + (1 - (cfg.ylog ? logPos(v, ylo, yhi) : (v - ylo) / (yhi - ylo))) * PH; };
    var g = "";
    // gridlines + ticks
    var xticks = [];
    if (cfg.xlog) {
      for (var e = Math.ceil(Math.log10(xlo)); e <= Math.log10(xhi); e++) xticks.push(Math.pow(10, e));
      if (cfg.xkind === "time") xticks = [10, 60, 300, 900, 1800, 3600].filter(function (v) { return v >= xlo && v <= xhi; });
    }
    xticks.forEach(function (v) {
      g += '<line x1="' + X(v) + '" y1="' + MT + '" x2="' + X(v) + '" y2="' + (MT + PH) + '" stroke="#30363d" stroke-width="0.5" opacity="0.6"/>';
      g += '<text x="' + X(v) + '" y="' + (H - MB + 16) + '" font-size="9.5" fill="#8b949e" text-anchor="middle">' + fmtTickX(v, cfg.xkind) + "</text>";
    });
    var yticks = cfg.ylog ? [10, 60, 300, 900, 1800].filter(function (v) { return v >= ylo && v <= yhi; }) : [20, 30, 40, 50, 60].filter(function (v) { return v >= ylo && v <= yhi; });
    yticks.forEach(function (v) {
      g += '<line x1="' + ML + '" y1="' + Y(v) + '" x2="' + (ML + PW) + '" y2="' + Y(v) + '" stroke="#30363d" stroke-width="0.5" opacity="0.6"/>';
      g += '<text x="' + (ML - 5) + '" y="' + (Y(v) + 3) + '" font-size="9.5" fill="#8b949e" text-anchor="end">' + (cfg.ylog ? fmtTickX(v, "time") : v) + "</text>";
    });
    g += '<line x1="' + ML + '" y1="' + (MT + PH) + '" x2="' + (ML + PW) + '" y2="' + (MT + PH) + '" stroke="#30363d"/><line x1="' + ML + '" y1="' + MT + '" x2="' + ML + '" y2="' + (MT + PH) + '" stroke="#30363d"/>';
    // bracket dividers from the reference design (cost tiers, time splits, II lines)
    (cfg.dividers || []).forEach(function (dv) {
      if (dv.axis === "x") {
        if (dv.v < xlo || dv.v > xhi) return;
        var dx = X(dv.v);
        g += '<line x1="' + dx + '" y1="' + MT + '" x2="' + dx + '" y2="' + (MT + PH) + '" stroke="#8b949e" stroke-width="1" stroke-dasharray="4 4" opacity="0.55"/>';
        if (dv.label) g += '<text x="' + (dx + 3) + '" y="' + (MT + 11) + '" font-size="9" fill="#8b949e">' + dv.label + "</text>";
      } else {
        if (dv.v < ylo || dv.v > yhi) return;
        var dy = Y(dv.v);
        g += '<line x1="' + ML + '" y1="' + dy + '" x2="' + (ML + PW) + '" y2="' + dy + '" stroke="#8b949e" stroke-width="1" stroke-dasharray="4 4" opacity="0.55"/>';
        if (dv.label) g += '<text x="' + (ML + 3) + '" y="' + (dy - 3) + '" font-size="9" fill="#8b949e">' + dv.label + "</text>";
      }
    });
    g += '<text x="' + (ML + PW / 2) + '" y="' + (H - 4) + '" font-size="10.5" fill="#8b949e" text-anchor="middle">' + cfg.xlabel + "</text>";
    g += '<text x="12" y="' + (MT + PH / 2) + '" font-size="10.5" fill="#8b949e" text-anchor="middle" transform="rotate(-90 12 ' + (MT + PH / 2) + ')">' + cfg.ylabel.replace("Intelligence Index", scoreAxis()) + "</text>";
    // effort ladder lines per family (kept readable: only when few points show)
    if (rows.length <= 60) {
      famList.forEach(function (fam) {
        if (S.off[fam]) return;
        var pts = rows.filter(function (p) { return p.family === fam; }).sort(function (a, b) { return eord(a.effort) - eord(b.effort); });
        if (pts.length < 2) return;
        var isDS = fam.indexOf("DeepSeek") === 0;
        g += '<polyline points="' + pts.map(function (p) { return X(cfg.x(p)).toFixed(1) + "," + Y(cfg.y(p)).toFixed(1); }).join(" ") + '" fill="none" stroke="' + famColor[fam] + '" stroke-width="' + (isDS ? 1.6 : 1) + '" opacity="' + (isDS ? 0.8 : 0.4) + '"' + (isDS ? ' stroke-dasharray="2.5 3"' : "") + "/>";
      });
    }
    // frontier curve for this chart's axis pair
    var fr = rows.filter(function (p) { return p[cfg.flag]; }).sort(function (a, b) { return cfg.x(a) - cfg.x(b); });
    if (fr.length > 1) {
      g += '<polyline points="' + fr.map(function (p) { return X(cfg.x(p)).toFixed(1) + "," + Y(cfg.y(p)).toFixed(1); }).join(" ") + '" fill="none" stroke="#e6edf3" stroke-width="1.4" opacity="0.65" stroke-dasharray="5 3"/>';
    }
    // dot sizing by the chart's third metric (reference design's toggle)
    var sizeVals = null, sizeLo = 0, sizeHi = 1;
    if (S.sizeBy[id] && cfg.sizeMetric) {
      sizeVals = rows.map(cfg.sizeMetric).filter(function (v) { return v != null; });
      if (sizeVals.length) {
        var tr = cfg.sizeLog ? sizeVals.map(function (v) { return Math.log10(v); }) : sizeVals;
        sizeLo = Math.min.apply(null, tr); sizeHi = Math.max.apply(null, tr);
      }
    }
    function radius(p, onF) {
      if (!sizeVals || !sizeVals.length || !cfg.sizeMetric) return onF ? 5.5 : 4.4;
      var v = cfg.sizeMetric(p);
      if (v == null) return 3;
      var t = cfg.sizeLog ? Math.log10(v) : v;
      var f = sizeHi > sizeLo ? Math.min(1, Math.max(0, (t - sizeLo) / (sizeHi - sizeLo))) : 0.5;
      return 1.6 + Math.pow(f, 1.5) * 8.4;
    }
    // 2026-11-01 (Drew, code#3335): every dot carries its family color;
    // frontier membership is emphasis (white ring, full opacity), not
    // the only color — on real data a chart's frontier can be a single
    // dominant point, which used to grey out the whole chart.
    rows.forEach(function (p) {
      var cx = X(cfg.x(p)), cy = Y(cfg.y(p));
      var onF = !!p[cfg.flag];
      var col = famColor[p.family];
      var r = radius(p, onF);
      var hollow = p.benchmark && p.benchmark.indexOf("AA") !== 0;
      var tip = p.family + " " + (p.effort || "") + " — " + scoreWord() + " " + fmtN(p.score, 1) + " · " + fmt$(p.cost) + "/task · " + fmtT(p.time) + "/task" + (p.tps != null ? " · " + p.tps + " tok/s" : "") + (hollow ? " · " + p.benchmark + " basis" : "");
      g += '<circle class="xpt" data-key="' + esc(p.key) + '" cx="' + cx.toFixed(1) + '" cy="' + cy.toFixed(1) + '" r="' + r.toFixed(1) + '" fill="' + col + '" fill-opacity="' + (hollow ? 0.25 : onF ? 0.95 : 0.45) + '" stroke="' + (S.sel === p.key || onF ? "#fff" : col) + '" stroke-width="' + (S.sel === p.key ? 2 : onF ? 1.8 : 1) + '"' + (hollow ? ' stroke-dasharray="2.5 1.8"' : "") + "><title>" + esc(tip) + "</title></circle>";
      if (onF || S.sel === p.key) {
        var anchorEnd = cx > ML + PW * 0.78;
        g += '<text data-key="' + esc(p.key) + '" x="' + (anchorEnd ? cx - r - 3 : cx + r + 3) + '" y="' + (cy + 3) + '" font-size="8.6" fill="' + (onF ? "#c9d1d9" : "#8b949e") + '" text-anchor="' + (anchorEnd ? "end" : "start") + '" style="cursor:pointer">' + esc(shortLabel(p)) + "</text>";
      }
    });
    svg.innerHTML = g;
    svg.querySelectorAll("[data-key]").forEach(function (el) {
      el.addEventListener("click", function () { selectPoint(el.getAttribute("data-key")); });
    });
  }
  var COST_DIVIDERS = [0.15, 0.75, 2.5].map(function (v, i) {
    return { axis: "x", v: v, label: ["Mid", "Premium", "Flagship"][i] };
  });
  var TIME_DIVIDERS_X = [60, 300, 900].map(function (v) { return { axis: "x", v: v, label: "" }; });
  var TIME_DIVIDERS_Y = [60, 300, 900].map(function (v) { return { axis: "y", v: v, label: "" }; });
  var CHARTS = [
    { id: "xpl-c1", x: function (p) { return p.time; }, y: function (p) { return p.score; }, xlog: true, ylog: false, xkind: "time", xlabel: "Seconds to finish one task (log)", ylabel: "Intelligence Index", flag: "frontierTime", need: "per-task time", sizeMetric: function (p) { return p.cost; }, sizeLog: true, dividers: TIME_DIVIDERS_X },
    { id: "xpl-c2", x: function (p) { return p.cost; }, y: function (p) { return p.score; }, xlog: true, ylog: false, xkind: "cost", xlabel: "Cost per task, USD (log)", ylabel: "Intelligence Index", flag: "frontierCost", need: "cost", sizeMetric: function (p) { return p.time; }, sizeLog: true, dividers: COST_DIVIDERS.concat([{ axis: "y", v: 40, label: "II 40" }, { axis: "y", v: 50, label: "II 50" }]) },
    { id: "xpl-c3", x: function (p) { return p.cost; }, y: function (p) { return p.time; }, xlog: true, ylog: true, xkind: "cost", xlabel: "Cost per task, USD (log)", ylabel: "Seconds per task (log)", flag: "frontierCostTime", need: "cost + per-task time", sizeMetric: function (p) { return p.score; }, sizeLog: false, dividers: COST_DIVIDERS.concat(TIME_DIVIDERS_Y) },
  ];
  var CHART_ORDER = { plan: ["xpl-c1", "xpl-c2", "xpl-c3"], api: ["xpl-c2", "xpl-c1", "xpl-c3"], pertask: ["xpl-c3", "xpl-c2", "xpl-c1"] };
  // --- chart expand + zoom/pan (#3298) -------------------------------
  // Every chart on this page is an SVG with a viewBox, so one uniform
  // layer covers all of them: the six explorer charts (tool buttons
  // are server-rendered next to each chart) and the band scatters
  // built by renderRecs / renderBandComparisons (tools injected
  // here). Zoom/pan manipulates only the viewBox attribute; draws
  // rewrite innerHTML (and, for the scores chart, the viewBox
  // height), so this layer tracks the last viewBox it applied — a
  // different attribute at enhance time means a draw reset the
  // chart and any zoom state for it is dropped. State is per chart;
  // Reset restores the drawn viewBox. Pan only acts while zoomed.
  var chartViews = {};
  function chartKeyFor(svg) {
    if (svg.id) return svg.id;
    if (!svg.getAttribute("data-zkey")) svg.setAttribute("data-zkey", "zx" + Math.random().toString(36).slice(2, 9));
    return svg.getAttribute("data-zkey");
  }
  function parseViewBox(s) {
    var p = (s || "").split(/\\s+/).map(Number);
    if (p.length !== 4 || p.some(function (n) { return !isFinite(n); })) return null;
    return p;
  }
  function chartState(svg) {
    var key = chartKeyFor(svg);
    var attr = svg.getAttribute("viewBox") || "";
    var st = chartViews[key];
    if (!st || st.applied !== attr) {
      var base = parseViewBox(attr);
      if (!base) return null;
      st = chartViews[key] = { base: base, cur: null, applied: attr };
    }
    return st;
  }
  function applyChartView(svg, st) {
    var v = st.cur || st.base;
    var s = v.join(" ");
    svg.setAttribute("viewBox", s);
    st.applied = s;
  }
  function clampChartView(st) {
    var b = st.base, c = st.cur;
    c[0] = Math.min(Math.max(c[0], b[0]), b[0] + b[2] - c[2]);
    c[1] = Math.min(Math.max(c[1], b[1]), b[1] + b[3] - c[3]);
  }
  function zoomChart(svg, factor, fx, fy) {
    var st = chartState(svg);
    if (!st) return;
    var cur = st.cur ? st.cur.slice() : st.base.slice();
    var nw = Math.min(Math.max(cur[2] * factor, st.base[2] / 16), st.base[2]);
    var nh = nw * st.base[3] / st.base[2];
    var cx = cur[0] + (fx == null ? 0.5 : fx) * cur[2];
    var cy = cur[1] + (fy == null ? 0.5 : fy) * cur[3];
    cur[0] = cx - (fx == null ? 0.5 : fx) * nw;
    cur[1] = cy - (fy == null ? 0.5 : fy) * nh;
    cur[2] = nw; cur[3] = nh;
    st.cur = nw >= st.base[2] ? null : cur;
    if (st.cur) clampChartView(st);
    applyChartView(svg, st);
  }
  function panChart(svg, dx, dy) {
    var st = chartState(svg);
    if (!st || !st.cur) return;
    var r = svg.getBoundingClientRect();
    if (!r.width || !r.height) return;
    st.cur[0] -= dx * st.cur[2] / r.width;
    st.cur[1] -= dy * st.cur[3] / r.height;
    clampChartView(st);
    applyChartView(svg, st);
  }
  function resetChart(svg) {
    var st = chartState(svg);
    if (!st) return;
    st.cur = null;
    applyChartView(svg, st);
  }
  function chartHost(svg) { return svg.closest(".chart") || svg.parentElement; }
  function setChartExpanded(svg, on) {
    var host = chartHost(svg);
    if (!host) return;
    host.classList.toggle("chart-expanded", on);
    var key = chartKeyFor(svg);
    document.querySelectorAll('[data-zact="expand"]').forEach(function (b) {
      if (b.getAttribute("data-zfor") === key) {
        b.textContent = on ? "✕ Close" : "⤢ Expand";
        b.setAttribute("aria-expanded", on ? "true" : "false");
      }
    });
  }
  function chartToolsHtml(key) {
    return '<span class="chart-tools" data-tools="' + key + '">' +
      '<button type="button" data-zact="expand" data-zfor="' + key + '">⤢ Expand</button>' +
      '<button type="button" data-zact="in" data-zfor="' + key + '" aria-label="Zoom in">+</button>' +
      '<button type="button" data-zact="out" data-zfor="' + key + '" aria-label="Zoom out">−</button>' +
      '<button type="button" data-zact="reset" data-zfor="' + key + '" aria-label="Reset view">⟲</button></span>';
  }
  function findChartSvg(key) {
    var byId = document.getElementById(key);
    if (byId) return byId;
    return document.querySelector('svg[data-zkey="' + key + '"]');
  }
  function enhanceChart(svg) {
    var st = chartState(svg);
    if (!st) return;
    if (st.cur) applyChartView(svg, st);
    if (svg.getAttribute("data-zbound")) return;
    svg.setAttribute("data-zbound", "1");
    svg.classList.add("chart-zoomable");
    // Band scatters are bare children of the shared #xpl-recs /
    // #xpl-bandgroups containers (Codex P2, PR #4154): give each a
    // dedicated wrapper so Expand isolates this chart instead of
    // overlaying the whole section.
    var zparent = svg.parentElement;
    if (zparent && (zparent.id === "xpl-recs" || zparent.id === "xpl-bandgroups")) {
      var mini = document.createElement("div");
      mini.className = "chart-mini";
      zparent.insertBefore(mini, svg);
      mini.appendChild(svg);
    }
    var key = chartKeyFor(svg);
    if (!document.querySelector('[data-tools="' + key + '"]')) {
      var host = chartHost(svg);
      if (host) {
        var tmp = document.createElement("span");
        tmp.innerHTML = chartToolsHtml(key);
        host.insertBefore(tmp.firstChild, svg);
      }
    }
    var drag = null;
    svg.addEventListener("pointerdown", function (e) {
      if (e.button !== 0) return;
      // No pointer capture yet (Codex P1, PR #4154): capturing here
      // would retarget an ordinary click away from the point/bar
      // selection handlers on the svg's descendants. Capture only
      // once pointermove detects an actual pan gesture.
      drag = { lx: e.clientX, ly: e.clientY, sx: e.clientX, sy: e.clientY, moved: false, id: e.pointerId };
    });
    svg.addEventListener("pointermove", function (e) {
      if (!drag) return;
      var dx = e.clientX - drag.lx, dy = e.clientY - drag.ly;
      drag.lx = e.clientX; drag.ly = e.clientY;
      if (!drag.moved && Math.abs(e.clientX - drag.sx) + Math.abs(e.clientY - drag.sy) > 4) {
        drag.moved = true;
        try { svg.setPointerCapture(drag.id); } catch (err) {}
      }
      if (drag.moved) panChart(svg, dx, dy);
    });
    svg.addEventListener("pointerup", function () {
      if (drag && drag.moved) svg.setAttribute("data-zsuppress", String(Date.now()));
      drag = null;
    });
    svg.addEventListener("click", function (e) {
      var t = Number(svg.getAttribute("data-zsuppress") || 0);
      if (t && Date.now() - t < 400) { e.stopPropagation(); e.preventDefault(); svg.removeAttribute("data-zsuppress"); }
    }, true);
    svg.addEventListener("wheel", function (e) {
      e.preventDefault();
      var r = svg.getBoundingClientRect();
      var fx = r.width ? (e.clientX - r.left) / r.width : 0.5;
      var fy = r.height ? (e.clientY - r.top) / r.height : 0.5;
      zoomChart(svg, e.deltaY < 0 ? 0.85 : 1.18, fx, fy);
    }, { passive: false });
  }
  function enhanceCharts() {
    document.querySelectorAll("#xpl-charts svg, #xpl-recs svg, #xpl-bandgroups svg").forEach(function (svg) { enhanceChart(svg); });
  }
  document.addEventListener("click", function (e) {
    var b = e.target.closest("[data-zact]");
    if (!b) return;
    var svg = findChartSvg(b.getAttribute("data-zfor"));
    if (!svg) return;
    var act = b.getAttribute("data-zact");
    if (act === "expand") {
      var host = chartHost(svg);
      setChartExpanded(svg, !(host && host.classList.contains("chart-expanded")));
    } else if (act === "in") zoomChart(svg, 0.8, null, null);
    else if (act === "out") zoomChart(svg, 1.25, null, null);
    else if (act === "reset") resetChart(svg);
  });
  document.addEventListener("keydown", function (e) {
    if (e.key !== "Escape") return;
    document.querySelectorAll(".chart-expanded").forEach(function (h) { h.classList.remove("chart-expanded"); });
    document.querySelectorAll('[data-zact="expand"]').forEach(function (b) { b.textContent = "⤢ Expand"; b.setAttribute("aria-expanded", "false"); });
  });
  function drawCharts() {
    var wrap = document.getElementById("xpl-charts");
    var order = CHART_ORDER[S.xp];
    wrap.querySelectorAll(".chart").forEach(function (c) {
      c.classList.remove("primary");
      var idx = order.indexOf(c.getAttribute("data-chart"));
      c.style.order = idx >= 0 ? String(idx + 1) : "9";
    });
    var first = wrap.querySelector('[data-chart="' + order[0] + '"]');
    if (first) first.classList.add("primary");
    CHARTS.forEach(function (c) { drawChart(c.id, c); });
    drawTasks();
    drawMultiples();
    drawScores();
  }
  function downloadFile(name, text, type) {
    var blob = new Blob([text], { type: type || "text/plain" });
    var url = URL.createObjectURL(blob);
    var a = document.createElement("a");
    a.download = name;
    a.href = url;
    a.click();
    setTimeout(function () { URL.revokeObjectURL(url); }, 500);
  }
  function downloadPng(svgId) {
    var svg = document.getElementById(svgId);
    if (!svg) return;
    var vb = (svg.getAttribute("viewBox") || "0 0 680 400").split(" ");
    var w = Math.round(Number(vb[2]) * 2), h = Math.round(Number(vb[3]) * 2);
    var clone = svg.cloneNode(true);
    clone.setAttribute("xmlns", "http://www.w3.org/2000/svg");
    clone.setAttribute("width", String(w));
    clone.setAttribute("height", String(h));
    var blob = new Blob([new XMLSerializer().serializeToString(clone)], { type: "image/svg+xml;charset=utf-8" });
    var url = URL.createObjectURL(blob);
    var img = new Image();
    img.onload = function () {
      var cv = document.createElement("canvas");
      cv.width = w; cv.height = h;
      var ctx = cv.getContext("2d");
      ctx.fillStyle = "#0d1117";
      ctx.fillRect(0, 0, w, h);
      ctx.drawImage(img, 0, 0, w, h);
      URL.revokeObjectURL(url);
      var a = document.createElement("a");
      a.download = "cost-explorer-" + svgId + ".png";
      a.href = cv.toDataURL("image/png");
      a.click();
    };
    img.src = url;
  }
  // --- chart 4: tasks completed vs duration (scatter-widget parity) ---
  var BUDGET_LADDER = [0.25, 1, 2, 5, 10, 20, 50, 100, 120, 200];
  function nominalBudget(cost) {
    var target = 30 * cost, best = BUDGET_LADDER[0], bd = Infinity;
    BUDGET_LADDER.forEach(function (b) {
      var d = Math.abs(Math.log10(b / target));
      if (d < bd) { bd = d; best = b; }
    });
    return best;
  }
  function drawTasks() {
    var svg = document.getElementById("xpl-c4");
    if (!svg) return;
    var all = visPoints();
    var rows = all.filter(function (p) { return p.cost != null && p.time != null; });
    var note = svg.parentElement.querySelector("[data-note]");
    if (note) {
      var missing = all.filter(function (p) { return p.cost == null || p.time == null; });
      note.textContent = rows.length + " of " + all.length + " shown — the rest lack a measured cost or per-task time: " +
        missing.slice(0, 4).map(shortLabel).join(", ") + (missing.length > 4 ? " (+" + (missing.length - 4) + " more)" : "") +
        ". Circles = a nominal budget ($0.25–$200, picked per model so its task count lands in range). ◆ = the model's published OpenCode Go monthly cap spent on that model alone (docs reading, 2026-10-20 — the fleet's workspaces appear to meter one pooled bar instead). Hollow = non-AA benchmark basis.";
    }
    document.querySelectorAll("[data-tmode]").forEach(function (b) {
      b.classList.toggle("active", b.getAttribute("data-tmode") === S.tasksMode);
    });
    if (!rows.length) { svg.innerHTML = ""; return; }
    var per = S.tasksMode === "per";
    var pts = rows.map(function (p) {
      var budget = nominalBudget(p.cost);
      var tasks = Math.max(1, Math.floor(budget / p.cost));
      var cap = goCapFor(p);
      return { p: p, budget: budget, tasks: tasks, y: per ? p.time : tasks * p.time, goTasks: cap ? Math.floor(cap / p.cost) : null, goY: cap ? (per ? p.time : Math.floor(cap / p.cost) * p.time) : null };
    });
    var xs = pts.map(function (t) { return t.tasks; }).concat(pts.filter(function (t) { return t.goTasks; }).map(function (t) { return t.goTasks; }));
    var ys = pts.map(function (t) { return t.y; }).concat(pts.filter(function (t) { return t.goY != null; }).map(function (t) { return t.goY; }));
    var xlo = Math.pow(10, Math.floor(Math.log10(Math.min.apply(null, xs)) * 2) / 2) / 1.4;
    var xhi = Math.max.apply(null, xs) * 1.6;
    var ylo = Math.pow(10, Math.floor(Math.log10(Math.min.apply(null, ys)) * 2) / 2) / 1.4;
    var yhi = Math.max.apply(null, ys) * 1.5;
    var X = function (v) { return ML + logPos(v, xlo, xhi) * PW; };
    var Y = function (v) { return MT + (1 - logPos(v, ylo, yhi)) * PH; };
    var g = "";
    [1, 3, 10, 30, 100, 300, 1000, 3000].filter(function (v) { return v >= xlo && v <= xhi; }).forEach(function (v) {
      g += '<line x1="' + X(v) + '" y1="' + MT + '" x2="' + X(v) + '" y2="' + (MT + PH) + '" stroke="#30363d" stroke-width="0.5" opacity="0.6"/><text x="' + X(v) + '" y="' + (H - MB + 16) + '" font-size="9.5" fill="#8b949e" text-anchor="middle">' + v + "</text>";
    });
    [10, 30, 100, 300, 1000, 3000, 10000, 43200, 140000].filter(function (v) { return v >= ylo && v <= yhi; }).forEach(function (v) {
      g += '<line x1="' + ML + '" y1="' + Y(v) + '" x2="' + (ML + PW) + '" y2="' + Y(v) + '" stroke="#30363d" stroke-width="0.5" opacity="0.6"/><text x="' + (ML - 5) + '" y="' + (Y(v) + 3) + '" font-size="9.5" fill="#8b949e" text-anchor="end">' + fmtT(v) + "</text>";
    });
    g += '<line x1="' + ML + '" y1="' + (MT + PH) + '" x2="' + (ML + PW) + '" y2="' + (MT + PH) + '" stroke="#30363d"/><line x1="' + ML + '" y1="' + MT + '" x2="' + ML + '" y2="' + (MT + PH) + '" stroke="#30363d"/>';
    g += '<text x="' + (ML + PW / 2) + '" y="' + (H - 4) + '" font-size="10.5" fill="#8b949e" text-anchor="middle">tasks completed (log scale)</text>';
    g += '<text x="12" y="' + (MT + PH / 2) + '" font-size="10.5" fill="#8b949e" text-anchor="middle" transform="rotate(-90 12 ' + (MT + PH / 2) + ')">' + (per ? "duration per task" : "total time, all tasks back-to-back") + "</text>";
    pts.forEach(function (t) {
      var p = t.p, cx = X(t.tasks), cy = Y(t.y), col = famColor[p.family];
      var hollow = p.benchmark && p.benchmark.indexOf("AA") !== 0;
      var tip = shortLabel(p) + " — $" + t.budget + " nominal → " + t.tasks + " tasks · " + fmtT(p.time) + "/task · " + fmt$(p.cost) + "/task · " + scoreWord() + " " + fmtN(p.score, 1) + (t.goTasks ? " · published Go cap buys " + fmtInt(t.goTasks) + " tasks" : "");
      g += '<circle class="xpt" data-key="' + esc(p.key) + '" cx="' + cx.toFixed(1) + '" cy="' + cy.toFixed(1) + '" r="5.5" fill="' + col + '" fill-opacity="' + (hollow ? 0.25 : 0.9) + '" stroke="' + (S.sel === p.key ? "#fff" : col) + '" stroke-width="' + (S.sel === p.key ? 2 : 1.5) + '"' + (hollow ? ' stroke-dasharray="2.5 1.8"' : "") + "><title>" + esc(tip) + "</title></circle>";
      if (t.goTasks) {
        var gx = X(t.goTasks), gy = Y(t.goY);
        g += '<rect data-key="' + esc(p.key) + '" x="' + (gx - 5).toFixed(1) + '" y="' + (gy - 5).toFixed(1) + '" width="10" height="10" transform="rotate(45 ' + gx.toFixed(1) + " " + gy.toFixed(1) + ')" fill="' + col + '" fill-opacity="0.95" stroke="#0d1117" stroke-width="1.2" style="cursor:pointer"><title>' + esc(tip) + "</title></rect>";
      }
      if (p.frontierCostTime || S.sel === p.key) {
        var anchorEnd = cx > ML + PW * 0.78;
        g += '<text data-key="' + esc(p.key) + '" x="' + (anchorEnd ? cx - 9 : cx + 9) + '" y="' + (cy + 3) + '" font-size="8.6" fill="#c9d1d9" text-anchor="' + (anchorEnd ? "end" : "start") + '" style="cursor:pointer">' + esc(shortLabel(p)) + "</text>";
      }
    });
    svg.innerHTML = g;
    svg.querySelectorAll("[data-key]").forEach(function (el) {
      el.addEventListener("click", function () { selectPoint(el.getAttribute("data-key")); });
    });
  }
  // --- chart 5: multiples of a baseline (multiples-widget parity) ---
  function baselinePoint() {
    var pool = POINTS.filter(function (p) { return p.cost != null && p.time != null && p.score != null && S.hidden.indexOf(p.key) < 0; });
    if (!pool.length) return null;
    var chosen = pool.find(function (p) { return p.key === S.baseline; });
    if (chosen) return chosen;
    var lunaLow = pool.find(function (p) { return p.family.indexOf("Luna") >= 0 && String(p.effort).toLowerCase() === "low"; });
    if (lunaLow) return lunaLow;
    return pool.slice().sort(function (a, b) { return a.cost - b.cost; })[0];
  }
  function buildBaselineOptions() {
    var sel = document.getElementById("xf-baseline");
    if (!sel) return;
    var pool = POINTS.filter(function (p) { return p.cost != null && p.time != null && p.score != null; })
      .sort(function (a, b) { return a.family < b.family ? -1 : a.family > b.family ? 1 : eord(a.effort) - eord(b.effort); });
    var base = baselinePoint();
    sel.innerHTML = pool.map(function (p) {
      return '<option value="' + esc(p.key) + '"' + (base && p.key === base.key ? " selected" : "") + ">" + esc(shortLabel(p)) + "</option>";
    }).join("");
  }
  function drawMultiples() {
    var svg = document.getElementById("xpl-c5");
    if (!svg) return;
    var base = baselinePoint();
    var note = svg.parentElement.querySelector("[data-note]");
    var all = visPoints();
    var rows = base ? all.filter(function (p) { return p.cost != null && p.time != null && p.score != null; }) : [];
    if (note) {
      note.textContent = base
        ? "Baseline: " + shortLabel(base) + " = 1× on all three measures (" + fmt$(base.cost) + "/task, " + fmtT(base.time) + "/task, " + scoreWord() + " " + fmtN(base.score, 1) + "). " + rows.length + " of " + all.length + " configurations have all three measures."
        : "No configuration with cost, time and score is visible — the multiples view needs all three.";
    }
    renderMultTable(base, rows);
    if (!base || !rows.length) { svg.innerHTML = ""; return; }
    var pts = rows.map(function (p) {
      return { p: p, cx: p.cost / base.cost, ty: p.time / base.time, sx: p.score / base.score };
    });
    var xs = pts.map(function (t) { return t.cx; }), ys = pts.map(function (t) { return t.ty; });
    var xlo = Math.min(0.8, Math.min.apply(null, xs) / 1.4), xhi = Math.max.apply(null, xs) * 1.5;
    var ylo = Math.min(0.8, Math.min.apply(null, ys) / 1.4), yhi = Math.max.apply(null, ys) * 1.5;
    var X = function (v) { return ML + logPos(v, xlo, xhi) * PW; };
    var Y = function (v) { return MT + (1 - logPos(v, ylo, yhi)) * PH; };
    var g = "";
    [1, 3, 10, 30, 100, 300, 1000].filter(function (v) { return v >= xlo && v <= xhi; }).forEach(function (v) {
      g += '<line x1="' + X(v) + '" y1="' + MT + '" x2="' + X(v) + '" y2="' + (MT + PH) + '" stroke="#30363d" stroke-width="0.5" opacity="0.6"/><text x="' + X(v) + '" y="' + (H - MB + 16) + '" font-size="9.5" fill="#8b949e" text-anchor="middle">' + v + "×</text>";
    });
    [1, 3, 10, 30, 100, 300].filter(function (v) { return v >= ylo && v <= yhi; }).forEach(function (v) {
      g += '<line x1="' + ML + '" y1="' + Y(v) + '" x2="' + (ML + PW) + '" y2="' + Y(v) + '" stroke="#30363d" stroke-width="0.5" opacity="0.6"/><text x="' + (ML - 5) + '" y="' + (Y(v) + 3) + '" font-size="9.5" fill="#8b949e" text-anchor="end">' + v + "×</text>";
    });
    if (xlo < 1 && xhi > 1) g += '<line x1="' + X(1) + '" y1="' + MT + '" x2="' + X(1) + '" y2="' + (MT + PH) + '" stroke="#58a6ff" stroke-width="1" opacity="0.55"/>';
    if (ylo < 1 && yhi > 1) g += '<line x1="' + ML + '" y1="' + Y(1) + '" x2="' + (ML + PW) + '" y2="' + Y(1) + '" stroke="#58a6ff" stroke-width="1" opacity="0.55"/>';
    g += '<line x1="' + ML + '" y1="' + (MT + PH) + '" x2="' + (ML + PW) + '" y2="' + (MT + PH) + '" stroke="#30363d"/><line x1="' + ML + '" y1="' + MT + '" x2="' + ML + '" y2="' + (MT + PH) + '" stroke="#30363d"/>';
    g += '<text x="' + (ML + PW / 2) + '" y="' + (H - 4) + '" font-size="10.5" fill="#8b949e" text-anchor="middle">cost multiple of the baseline (log scale)</text>';
    g += '<text x="12" y="' + (MT + PH / 2) + '" font-size="10.5" fill="#8b949e" text-anchor="middle" transform="rotate(-90 12 ' + (MT + PH / 2) + ')">duration multiple of the baseline (lower = faster)</text>';
    pts.forEach(function (t) {
      var p = t.p, cx = X(t.cx), cy = Y(t.ty), col = famColor[p.family];
      var hollow = p.benchmark && p.benchmark.indexOf("AA") !== 0;
      var tip = shortLabel(p) + " — costs " + fmtN(t.cx, 2) + "× baseline (" + fmt$(p.cost) + "/task), takes " + fmtN(t.ty, 2) + "× as long (" + fmtT(p.time) + "/task), scores " + fmtN(t.sx, 2) + "× (" + scoreWord() + " " + fmtN(p.score, 1) + ")";
      g += '<circle class="xpt" data-key="' + esc(p.key) + '" cx="' + cx.toFixed(1) + '" cy="' + cy.toFixed(1) + '" r="5.5" fill="' + col + '" fill-opacity="' + (hollow ? 0.25 : 0.9) + '" stroke="' + (S.sel === p.key ? "#fff" : col) + '" stroke-width="' + (S.sel === p.key ? 2 : 1.5) + '"' + (hollow ? ' stroke-dasharray="2.5 1.8"' : "") + "><title>" + esc(tip) + "</title></circle>";
      if (p.key === base.key) g += '<circle cx="' + cx.toFixed(1) + '" cy="' + cy.toFixed(1) + '" r="10" fill="none" stroke="#58a6ff" stroke-width="1.5"/>';
      if (p.frontierCostTime || S.sel === p.key || p.key === base.key) {
        var anchorEnd = cx > ML + PW * 0.78;
        g += '<text data-key="' + esc(p.key) + '" x="' + (anchorEnd ? cx - 9 : cx + 9) + '" y="' + (cy + 3) + '" font-size="8.6" fill="#c9d1d9" text-anchor="' + (anchorEnd ? "end" : "start") + '" style="cursor:pointer">' + esc(shortLabel(p)) + "</text>";
      }
    });
    svg.innerHTML = g;
    svg.querySelectorAll("[data-key]").forEach(function (el) {
      el.addEventListener("click", function () { selectPoint(el.getAttribute("data-key")); });
    });
  }
  function renderMultTable(base, rows) {
    var tb = document.getElementById("xpl-mtrows");
    if (!tb) return;
    if (!base) { tb.innerHTML = ""; return; }
    var sorted = rows.slice().sort(function (a, b) { return a.cost / base.cost - b.cost / base.cost; });
    tb.innerHTML = sorted.map(function (p) {
      return '<tr data-sel="' + esc(p.key) + '" data-key="' + esc(p.key) + '"><td><strong>' + esc(p.family) + "</strong></td><td>" + esc(p.effort || "—") + '</td><td class="num">' + fmtN(p.score, 1) + '</td><td class="num">' + fmtN(p.score / base.score, 2) + '×</td><td class="num xpl-costcol">' + fmt$(p.cost) + '</td><td class="num xpl-costcol">' + fmtN(p.cost / base.cost, 2) + '×</td><td class="num">' + fmtT(p.time) + '</td><td class="num">' + fmtN(p.time / base.time, 2) + '×</td><td>' + esc(p.benchmark || "—") + "</td></tr>";
    }).join("");
    tb.querySelectorAll("tr").forEach(function (tr) {
      tr.addEventListener("click", function () { selectPoint(tr.getAttribute("data-key")); });
    });
    markSelectedRow();
  }
  // --- chart 6: scores only, incl. configurations with no published cost ---
  function drawScores() {
    var svg = document.getElementById("xpl-c6");
    if (!svg) return;
    var all = visPoints();
    var rows = all.filter(function (p) { return p.score != null; }).sort(function (a, b) { return b.score - a.score; });
    var note = svg.parentElement.querySelector("[data-note]");
    var top = rows.slice(0, 40);
    if (note) {
      note.textContent = "Top " + top.length + " of " + rows.length + " scored configurations (of " + all.length + " visible). Score-only view: configurations with no published cost belong here even though no cost chart can plot them.";
    }
    if (!top.length) { svg.innerHTML = ""; return; }
    var rowH = 17, topPad = 8, botPad = 22;
    var hgt = topPad + top.length * rowH + botPad;
    svg.setAttribute("viewBox", "0 0 680 " + hgt);
    var labelW = 218, plotW = 680 - labelW - 52;
    var maxScore = Math.max.apply(null, top.map(function (p) { return p.score; })) * 1.05;
    var g = "";
    top.forEach(function (p, i) {
      var y = topPad + i * rowH;
      var w = Math.max(2, (p.score / maxScore) * plotW);
      var col = famColor[p.family];
      var label = shortLabel(p) + (p.cost == null ? " (cost n/a)" : "");
      g += '<text data-key="' + esc(p.key) + '" x="' + (labelW - 6) + '" y="' + (y + 11) + '" font-size="9.5" fill="#c9d1d9" text-anchor="end" style="cursor:pointer">' + esc(label) + "</text>";
      g += '<rect data-key="' + esc(p.key) + '" x="' + labelW + '" y="' + (y + 2) + '" width="' + w.toFixed(1) + '" height="11" fill="' + col + '" fill-opacity="' + (S.sel === p.key ? 1 : 0.75) + '" style="cursor:pointer"><title>' + esc(shortLabel(p) + " — " + scoreWord() + " " + fmtN(p.score, 1) + " · " + fmt$(p.cost) + "/task · " + fmtT(p.time) + "/task") + "</title></rect>";
      g += '<text x="' + (labelW + w + 5) + '" y="' + (y + 11) + '" font-size="9.5" fill="#8b949e">' + fmtN(p.score, 1) + "</text>";
    });
    svg.innerHTML = g;
    svg.querySelectorAll("[data-key]").forEach(function (el) {
      el.addEventListener("click", function () { selectPoint(el.getAttribute("data-key")); });
    });
  }
  // --- hidden tray (reference design's hide-a-configuration flow) ---
  function renderTray() {
    var tray = document.getElementById("xpl-tray");
    if (!tray) return;
    var chips = document.getElementById("xpl-traychips");
    if (!S.hidden.length) { tray.hidden = true; chips.innerHTML = ""; return; }
    tray.hidden = false;
    chips.innerHTML = "";
    S.hidden.forEach(function (key) {
      var p = POINTS.find(function (x) { return x.key === key; });
      if (!p) return;
      var s = document.createElement("span");
      s.className = "hchip";
      s.textContent = shortLabel(p);
      var b = document.createElement("button");
      b.type = "button";
      b.textContent = "×";
      b.title = "Bring back";
      b.addEventListener("click", function () {
        S.hidden = S.hidden.filter(function (h) { return h !== key; });
        render();
      });
      s.appendChild(b);
      chips.appendChild(s);
    });
  }
  function renderLegend() {
    var lg = document.getElementById("xpl-legend");
    if (!lg) return;
    lg.innerHTML = "";
    famList.forEach(function (fam) {
      var b = document.createElement("button");
      b.type = "button";
      b.className = "xchip" + (S.off[fam] ? " off" : "");
      b.innerHTML = '<span class="dot" style="background:' + famColor[fam] + '"></span>' + esc(fam);
      b.addEventListener("click", function () {
        if (S.off[fam]) delete S.off[fam]; else S.off[fam] = true;
        render();
      });
      lg.appendChild(b);
    });
  }
  function showDetail(html) {
    var box = document.getElementById("xpl-detail");
    if (!box) return;
    if (!html) { box.style.display = "none"; box.innerHTML = ""; return; }
    box.innerHTML = html;
    box.style.display = "block";
  }
  function selectPoint(key) {
    S.sel = key;
    var p = POINTS.find(function (x) { return x.key === key; });
    if (!p) return;
    var beaten = p.beatenBy && p.beatenBy.length
      ? p.beatenBy.slice(0, 4).join("; ") + (p.beatenBy.length > 4 ? " (+" + (p.beatenBy.length - 4) + " more)" : "")
      : "nothing on all three axes — it is on the frontier";
    var budgets = p.cost
      ? "<br>Tasks per budget at this cost: $60 → " + fmtInt(tasksAt(p.cost, 60)) + " · $90 → " + fmtInt(tasksAt(p.cost, 90)) + " · $120 → " + fmtInt(tasksAt(p.cost, 120)) + " · $180 → " + fmtInt(tasksAt(p.cost, 180))
      : "";
    var cap = goCapFor(p);
    var goLine = cap && p.cost
      ? '<br><span class="muted">Published OpenCode Go cap for this model: $' + cap + "/mo → ~" + fmtInt(Math.floor(cap / p.cost)) + " tasks at this AA cost (docs reading — the fleet's workspaces appear to meter pooled).</span>"
      : "";
    var caiLine = "";
    if (p.harness != null || p.components) {
      var pct = function (v) { return v == null ? "—" : fmtN(v * 100, 1) + "%"; };
      caiLine = "<br>" +
        (p.harness ? "Harness: " + esc(p.harness) + " · " : "") +
        (p.tokensPerTask != null ? fmtInt(p.tokensPerTask) + " tokens/task" : "") +
        (p.components ? "<br>Components: DeepSWE " + pct(p.components.deepSwe) + " · Terminal-Bench " + pct(p.components.terminalBench) + " · SWE-Atlas-QnA " + pct(p.components.sweAtlas) : "");
    }
    showDetail(
      "<strong>" + esc(p.family) + " — " + esc(p.effort || "—") + "</strong> <span class='muted'>" + esc(p.creator) + "</span><br>" +
      scoreWord() + " " + fmtN(p.score, 1) + " · " + fmt$(p.cost) + "/task · " + fmtT(p.time) + " per task" +
      (p.ttft != null ? " · TTFT " + fmtT(p.ttft) : "") + (p.tps != null ? " · " + p.tps + " tok/s" : "") + budgets + "<br>" +
      '<span class="muted">Beaten on score + cost + time at once by: ' + esc(beaten) + "</span>" + caiLine + goLine +
      '<br><button type="button" id="xpl-hide" class="xpl-ctl-btn">Hide this configuration</button>',
    );
    var hideBtn = document.getElementById("xpl-hide");
    if (hideBtn) {
      hideBtn.addEventListener("click", function () {
        if (S.hidden.indexOf(p.key) < 0) S.hidden.push(p.key);
        S.sel = null;
        showDetail(null);
        render();
      });
    }
    drawCharts();
    markSelectedRow();
  }
  function selectCatalog(id) {
    S.sel = id;
    var e = (CATALOG || []).find(function (x) { return x.id === id; });
    if (!e) return;
    showDetail(
      "<strong>" + esc(e.name) + "</strong> <span class='muted'><code>" + esc(e.id) + "</code></span><br>" +
      (e.plan ? "Plan: <strong>" + esc(e.plan) + "</strong>" + (e.multiplier ? " — " + e.multiplier + "× list value" : "") + "<br>" : "") +
      (e.planNote ? '<span class="muted">' + esc(e.planNote) + "</span><br>" : "") +
      "Pool seats: " + (e.routes.length ? e.routes.map(function (r) { return "<code>" + esc(r) + "</code>"; }).join(" ") : '<span class="muted">none — no route sends it work</span>') +
      (e.goCapUsd != null && e.aaCostPerTask ? '<br><span class="muted">Published OpenCode Go cap: $' + e.goCapUsd + "/mo → ~" + fmtInt(Math.floor(e.goCapUsd / e.aaCostPerTask)) + " AA tasks at the measured cost (docs reading — the fleet's workspaces appear to meter pooled).</span>" : "") +
      (e.disabled ? '<br><span class="muted">Provider is disabled in the fleet config — catalogued but not callable.</span>' : "") +
      (function () {
        var proj = project(e);
        if (!proj.band) return "";
        var P = S.pj;
        return '<br><span class="muted">Your projected task (ESTIMATED, NOT MEASURED): ' + P.turns + " turns × " + fmtInt(Number(P.inputTokens) || 0) + " input + " + fmtInt(Number(P.answerTokens) || 0) + " answer + " + fmtInt(Number(P.thinkingTokens) || 0) + " thinking tokens/turn · " + P.cacheReadPct + "% cache reads · rate window: " + esc(proj.band.window || "flat") + " (" + fmt$(proj.band.input) + " in / " + fmt$(proj.band.output) + " out per 1M) → " + (proj.cost != null ? "~" + fmt$(proj.cost) + "/task" : "cost unknown — a required rate is not recorded") + (proj.time != null ? " · ~" + fmtT(proj.time) + "/task" : "") + ".</span>";
      })(),
    );
    markSelectedRow();
  }
  function markSelectedRow() {
    document.querySelectorAll(".xpl-table tr").forEach(function (tr) {
      tr.style.background = tr.getAttribute("data-sel") === S.sel ? "#1f6feb22" : "";
    });
  }
  // --- Pareto frontier tables (Site 1's Pareto frontiers tab) ------------
  // Each list keeps variants not dominated on its named dimensions;
  // percentile ranks use the current filtered population (100% = best).
  function pfBind(tb) {
    if (!tb) return;
    tb.querySelectorAll("tr").forEach(function (tr) {
      tr.addEventListener("click", function () { selectPoint(tr.getAttribute("data-key")); });
    });
  }
  function pfMetricCell(v, pct, fmt) {
    return v == null ? "—" : fmt(v) + (pct != null ? ' <span class="muted">· ' + pct + "%</span>" : "");
  }
  function pfNameCell(p) {
    return '<td><strong>' + esc(p.family) + '</strong> <span class="muted">' + esc(p.effort || "") + (p.harness ? " · " + esc(p.harness) : "") + "</span></td>";
  }
  function renderPareto() {
    var rows = visPoints();
    var scores = rows.map(function (p) { return p.score; });
    var times = rows.map(function (p) { return p.time; });
    var costs = rows.map(function (p) { return p.cost; });
    function stdRow(p) {
      return '<tr data-sel="' + esc(p.key) + '" data-key="' + esc(p.key) + '">' + pfNameCell(p) +
        '<td class="num">' + pfMetricCell(p.score, pctRank(scores, p.score, true), function (v) { return fmtN(v, 1); }) + "</td>" +
        '<td class="num">' + pfMetricCell(p.time, pctRank(times, p.time, false), fmtT) + "</td>" +
        '<td class="num xpl-costcol">' + pfMetricCell(p.cost, pctRank(costs, p.cost, false), fmt$) + "</td></tr>";
    }
    var gScore = function (p) { return p.score; }, gTime = function (p) { return p.time; }, gCost = function (p) { return p.cost; };
    var ssFrontier = paretoSet(rows, [{ get: gScore, better: "high" }, { get: gTime, better: "low" }]);
    var byScore = ssFrontier.slice().sort(function (a, b) { return cmpDesc(a.score, b.score) || cmpAsc(a.time, b.time); });
    var bySpeed = ssFrontier.slice().sort(function (a, b) { return cmpAsc(a.time, b.time) || cmpDesc(a.score, b.score); });
    var cut = Number(S.pfCut || 50);
    var three = paretoSet(rows, [{ get: gScore, better: "high" }, { get: gCost, better: "low" }, { get: gTime, better: "low" }])
      .filter(function (p) {
        if (!cut) return true;
        var ps = pctRank(scores, p.score, true), pt = pctRank(times, p.time, false);
        return ps != null && pt != null && ps >= cut && pt >= cut;
      })
      .sort(function (a, b) { return cmpAsc(a.cost, b.cost) || cmpDesc(a.score, b.score); });
    var psFrontier = paretoSet(rows, [{ get: gCost, better: "low" }, { get: gTime, better: "low" }])
      .sort(function (a, b) { return cmpAsc(a.cost, b.cost) || cmpAsc(a.time, b.time); });
    var tb;
    tb = document.getElementById("xpl-pf-score"); if (tb) { tb.innerHTML = byScore.map(stdRow).join(""); pfBind(tb); }
    tb = document.getElementById("xpl-pf-speed"); if (tb) { tb.innerHTML = bySpeed.map(stdRow).join(""); pfBind(tb); }
    tb = document.getElementById("xpl-pf-price"); if (tb) { tb.innerHTML = three.map(stdRow).join(""); pfBind(tb); }
    tb = document.getElementById("xpl-pf-ps");
    if (tb) {
      tb.innerHTML = psFrontier.map(function (p) {
        return '<tr data-sel="' + esc(p.key) + '" data-key="' + esc(p.key) + '">' + pfNameCell(p) +
          '<td class="num">' + pfMetricCell(p.time, pctRank(times, p.time, false), fmtT) + "</td>" +
          '<td class="num xpl-costcol">' + pfMetricCell(p.cost, pctRank(costs, p.cost, false), fmt$) + "</td>" +
          '<td class="num">' + (p.score == null ? "—" : fmtN(p.score, 1)) + "</td></tr>";
      }).join("");
      pfBind(tb);
    }
    var note = document.getElementById("xpl-pf-count");
    if (note) {
      note.textContent = "Score–speed frontier: " + ssFrontier.length + " variants · three-way frontier" +
        (cut ? " (score and speed at or above the " + cut + "th percentile)" : " (no percentile cutoff)") + ": " + three.length +
        " · price–speed trade-offs: " + psFrontier.length + ". Missing measures are excluded only from a frontier that needs them.";
    }
    markSelectedRow();
  }
  // --- Recommendations (Site 1's shortlist builder) ------------------------
  // Score bands are non-overlapping, from the visible population, before
  // tradeoff limits. Within a band: absolute limits first, then relative
  // price/speed limits against the cheapest and fastest near-top models;
  // survivors rank by (relative time)^speed-weight x (relative
  // price)^(1-speed-weight); scores inside the window break ties.
  function computeRecs() {
    return computeRecommendations(visPoints(), S.rec, S.bandCount, scoreWord());
  }
  function bandScatter(band) {
    var pts = band.members.filter(function (p) { return p.cost != null && p.time != null; });
    if (pts.length < 2) return "";
    var pickKeys = {};
    band.picks.forEach(function (r) { pickKeys[r.p.key] = true; });
    var bw = 330, bh = 180, ml = 44, mr = 8, mt = 8, mb = 24, pw = bw - ml - mr, ph = bh - mt - mb;
    var xs = pts.map(function (p) { return p.cost; }), ys = pts.map(function (p) { return p.time; });
    var xlo = Math.pow(10, Math.floor(Math.log10(Math.min.apply(null, xs)) * 2) / 2) / 1.4, xhi = Math.max.apply(null, xs) * 1.5;
    var ylo = Math.pow(10, Math.floor(Math.log10(Math.min.apply(null, ys)) * 2) / 2) / 1.4, yhi = Math.max.apply(null, ys) * 1.5;
    // Faster is DOWN: screen y grows with time (no inversion).
    var X = function (v) { return ml + logPos(v, xlo, xhi) * pw; };
    var Y = function (v) { return mt + logPos(v, ylo, yhi) * ph; };
    var g = '<svg viewBox="0 0 ' + bw + " " + bh + '" role="img" aria-label="Price versus speed in this score band" style="max-width:340px;width:100%">';
    g += '<line x1="' + ml + '" y1="' + (mt + ph) + '" x2="' + (ml + pw) + '" y2="' + (mt + ph) + '" stroke="#30363d"/><line x1="' + ml + '" y1="' + mt + '" x2="' + ml + '" y2="' + (mt + ph) + '" stroke="#30363d"/>';
    g += '<text x="' + (ml + pw / 2) + '" y="' + (bh - 4) + '" font-size="9" fill="#8b949e" text-anchor="middle">USD / task (log) — cheaper is left</text>';
    g += '<text x="10" y="' + (mt + ph / 2) + '" font-size="9" fill="#8b949e" text-anchor="middle" transform="rotate(-90 10 ' + (mt + ph / 2) + ')">time — faster is down</text>';
    pts.forEach(function (p) {
      var on = !!pickKeys[p.key], col = famColor[p.family];
      g += '<circle class="xpt" data-key="' + esc(p.key) + '" cx="' + X(p.cost).toFixed(1) + '" cy="' + Y(p.time).toFixed(1) + '" r="' + (on ? 5 : 3.6) + '" fill="' + col + '" fill-opacity="' + (on ? 0.95 : 0.4) + '" stroke="' + (on ? "#fff" : col) + '" stroke-width="' + (on ? 1.6 : 0.8) + '"><title>' + esc(shortLabel(p) + " — " + fmt$(p.cost) + "/task · " + fmtT(p.time)) + "</title></circle>";
    });
    return g + "</svg>";
  }
  function renderRecs() {
    var wrap = document.getElementById("xpl-recs");
    if (!wrap) return;
    var out = computeRecs();
    if (out.empty) { wrap.innerHTML = '<p class="muted">' + esc(out.empty) + "</p>"; return; }
    var R = S.rec;
    var summary = document.getElementById("xpl-rec-summary");
    var totalPicks = out.bands.reduce(function (a, b) { return a + b.picks.length; }, 0);
    var labs = {};
    out.bands.forEach(function (b) { b.picks.forEach(function (r) { labs[r.p.creator] = true; }); });
    if (summary) {
      summary.textContent = totalPicks + " choices across " + out.bands.length + " score bands · " + Object.keys(labs).length + " labs. Near-top: " + R.allow +
        " points · Speed priority: " + R.speedPriority + "% · At most " + R.slowAllow + "× slower · Price allowance: +" + R.priceAllow + "% · " +
        (R.diversity === "lab" ? "One per lab" : R.diversity === "labsfirst" ? "Different labs first" : "No lab grouping") + ".";
    }
    wrap.innerHTML = out.bands.map(function (band, bi) {
      var ref = band.ref;
      var cards = band.picks.map(function (r, ri) {
        var p = r.p;
        var vsRef = "";
        if (ref && ref.key !== p.key) {
          var dt = ref.time ? Math.round((1 - p.time / ref.time) * 1000) / 10 : null;
          var dc = ref.cost ? Math.round((1 - p.cost / ref.cost) * 1000) / 10 : null;
          vsRef = '<br><span class="muted">vs band-best ' + esc(shortLabel(ref)) + ': ' +
            (dt != null ? (dt >= 0 ? dt + "% less time" : Math.abs(dt) + "% more time") : "") +
            (dt != null && dc != null ? " · " : "") +
            (dc != null ? (dc >= 0 ? dc + "% less cost" : Math.abs(dc) + "% more cost") : "") + "</span>";
        } else if (ref) {
          vsRef = '<br><span class="muted">The top-scoring reference for this band.</span>';
        }
        var bestFactor = band.picks.length ? band.picks[0].factor : 1;
        return '<div class="card"><span class="badge ' + (ri === 0 ? "open" : "src") + '">' + (ri === 0 ? "Best fit" : p.creator === band.picks[0].p.creator ? "Alternative" : "Alternative lab") + "</span> " +
          '<button type="button" class="xpl-linkbtn" data-pick="' + esc(p.key) + '"><strong>' + esc(shortLabel(p)) + "</strong></button>" +
          '<span class="muted"> · ' + esc(p.creator) + (p.harness ? " · " + esc(p.harness) : "") + "</span><br>" +
          scoreWord() + " " + fmtN(p.score, 1) + " · " + fmt$(p.cost) + "/task · " + fmtT(p.time) + "/task · " +
          fmtN(band.best - p.score, 1) + " below band best · " + fmtN(r.factor / bestFactor, 2) + "× best tradeoff" + vsRef + "</div>";
      }).join("");
      var counts = {};
      band.members.forEach(function (p) { var d = band.decision[p.key] || "Not selected"; counts[d] = (counts[d] || 0) + 1; });
      var whyRows = band.members.slice().sort(function (a, b) {
        var ra = band.decision[a.key] === "Recommended" ? 0 : 1, rb = band.decision[b.key] === "Recommended" ? 0 : 1;
        return ra - rb || cmpDesc(a.score, b.score);
      }).map(function (p) {
        return '<tr data-key="' + esc(p.key) + '"><td><strong>' + esc(p.family) + '</strong> <span class="muted">' + esc(p.effort || "") + '</span></td><td>' + esc(p.creator) + "</td><td>" + esc(band.decision[p.key] || "Not selected") + "</td></tr>";
      }).join("");
      var whyCounts = Object.keys(counts).map(function (k) { return counts[k] + " · " + k; }).join(" &nbsp;·&nbsp; ");
      return "<h3>" + esc(band.label) + " · " + band.members.length + " variants</h3>" +
        '<p class="muted">Require score ≥ ' + fmtN(band.floor, 1) + " (band best " + fmtN(band.best, 1) + ", allowance " + R.allow + " points)" +
        (band.priceCeil != null ? " · Relative price ceiling " + fmt$(band.priceCeil) + " · Relative time ceiling " + fmtT(band.timeCeil) : " · no candidate carries all three measures") + "</p>" +
        bandScatter(band) +
        '<div class="board">' + (cards || '<p class="muted">No recommendation survives the limits of this band.</p>') + "</div>" +
        '<details><summary>Why these choices?</summary><p class="muted">' + whyCounts + '</p><div class="tblwrap"><table class="xpl-table"><caption>Why these choices — decisions by model and effort</caption><thead><tr><th scope="col">Model / effort</th><th scope="col">Lab</th><th scope="col">Decision</th></tr></thead><tbody>' + whyRows + "</tbody></table></div></details>";
    }).join("");
    wrap.querySelectorAll("[data-pick]").forEach(function (b) {
      b.addEventListener("click", function () { selectPoint(b.getAttribute("data-pick")); });
    });
    wrap.querySelectorAll("tbody tr").forEach(function (tr) {
      tr.addEventListener("click", function () { selectPoint(tr.getAttribute("data-key")); });
    });
    wrap.querySelectorAll("svg [data-key]").forEach(function (el) {
      el.addEventListener("click", function () { selectPoint(el.getAttribute("data-key")); });
    });
    markSelectedRow();
    // Recommendation inputs redraw #xpl-recs directly, without
    // going through render() (Codex P2, PR #4154) — re-enhance here
    // so the band charts keep their expand/zoom/pan controls.
    enhanceCharts();
  }
  // --- Band comparisons (Site 1's six banded shortlists) -------------------
  // Near-leader windows at 1x: score within 2 points, time within 10%,
  // price within 25% of the band leader; the scale select multiplies all
  // three. Lane charts visualize the same near-leader choices as the
  // comparison tables; they do not recalculate band membership.
  function logEdges(vals, n) {
    var xs = vals.filter(function (v) { return v != null && v > 0; });
    if (!xs.length) return [];
    var mn = Math.min.apply(null, xs), mx = Math.max.apply(null, xs);
    if (mn === mx) return [mn, mx];
    var edges = [];
    for (var i = 0; i <= n; i++) edges.push(mn * Math.pow(mx / mn, i / n));
    return edges;
  }
  function scoreEdges(rows, n) {
    var xs = rows.map(function (p) { return p.score; }).filter(function (v) { return v != null; });
    if (!xs.length) return [];
    var mn = Math.min.apply(null, xs), mx = Math.max.apply(null, xs);
    var edges = [];
    for (var i = 0; i <= n; i++) edges.push(mn + ((mx - mn) * i) / n);
    return edges;
  }
  function bandIndex(v, edges) {
    for (var i = 0; i < edges.length - 1; i++) {
      if (v >= edges[i] && (i === edges.length - 2 ? v <= edges[i + 1] : v < edges[i + 1])) return i;
    }
    return -1;
  }
  function chipFor(p, leader) {
    var notes = [];
    if (leader && leader.key !== p.key) {
      var better = [];
      if (p.score != null && leader.score != null && p.score > leader.score) better.push("higher score");
      if (p.cost != null && leader.cost != null && p.cost < leader.cost) better.push("lower price");
      if (p.time != null && leader.time != null && p.time < leader.time) better.push("faster");
      if (better.length) notes.push("Better at: " + better.join(", "));
    }
    return '<button type="button" class="xchip" data-pick="' + esc(p.key) + '">' + esc(shortLabel(p)) + " — " + fmtN(p.score, 1) + " · " + fmt$(p.cost) + " · " + fmtT(p.time) + (notes.length ? ' <span class="muted">(' + esc(notes.join("; ")) + ")</span>" : "") + "</button>";
  }
  function bandRangeLine(members) {
    var withScore = members.filter(function (p) { return p.score != null; });
    var withTime = members.filter(function (p) { return p.time != null; });
    var parts = [];
    if (withScore.length) {
      var sMin = withScore.reduce(function (a, b) { return b.score < a.score ? b : a; });
      var sMax = withScore.reduce(function (a, b) { return b.score > a.score ? b : a; });
      parts.push("score " + fmtN(sMin.score, 1) + " (" + shortLabel(sMin) + ") – " + fmtN(sMax.score, 1) + " (" + shortLabel(sMax) + ")");
    }
    if (withTime.length) {
      var tMin = withTime.reduce(function (a, b) { return b.time < a.time ? b : a; });
      var tMax = withTime.reduce(function (a, b) { return b.time > a.time ? b : a; });
      parts.push("time " + fmtT(tMin.time) + " (" + shortLabel(tMin) + ") – " + fmtT(tMax.time) + " (" + shortLabel(tMax) + ")");
    }
    return parts.join(" · ");
  }
  function bandLaneChart(title, metric, axisLabel, bands) {
    var lanes = bands.filter(function (b) { return b.choices && b.choices.length; });
    var values = [];
    lanes.forEach(function (b) {
      b.members.forEach(function (p) {
        var v = p[metric];
        if (v != null && isFinite(v) && (metric === "score" || v > 0)) values.push(v);
      });
    });
    if (!values.length) return "";
    var lo = Math.min.apply(null, values), hi = Math.max.apply(null, values);
    if (lo === hi) {
      if (metric === "score") { lo -= 1; hi += 1; }
      else { lo /= 1.4; hi *= 1.4; }
    }
    var logarithmic = metric !== "score";
    var logLo = logarithmic ? Math.log(lo) : lo;
    var logHi = logarithmic ? Math.log(hi) : hi;
    var width = 760, left = 232, right = 16, top = 28, row = 30, bottom = 30;
    var plotWidth = width - left - right;
    var height = top + lanes.length * row + bottom;
    function x(v) {
      var n = logarithmic ? Math.log(v) : v;
      return left + ((n - logLo) / (logHi - logLo)) * plotWidth;
    }
    function valueLabel(v) {
      return metric === "cost" ? fmt$(v) : metric === "time" ? fmtT(v) : fmtN(v, 1);
    }
    var svg = '<svg class="xpl-band-lanes" viewBox="0 0 ' + width + " " + height + '" role="img" aria-label="' + esc(title + " lane chart; " + axisLabel) + '" style="max-width:760px;width:100%;height:auto">';
    svg += '<text x="' + left + '" y="13" font-size="10" fill="#8b949e">' + esc(axisLabel) + (logarithmic ? " (log scale)" : " (linear scale)") + "</text>";
    for (var ti = 0; ti <= 4; ti++) {
      var fraction = ti / 4;
      var tv = logarithmic ? Math.exp(logLo + (logHi - logLo) * fraction) : lo + (hi - lo) * fraction;
      var tx = left + plotWidth * fraction;
      svg += '<line x1="' + tx.toFixed(1) + '" y1="' + (top - 4) + '" x2="' + tx.toFixed(1) + '" y2="' + (height - bottom + 2) + '" stroke="#30363d" stroke-dasharray="2 3"/>';
      svg += '<text x="' + tx.toFixed(1) + '" y="' + (height - 8) + '" font-size="9" fill="#8b949e" text-anchor="middle">' + esc(valueLabel(tv)) + "</text>";
    }
    lanes.forEach(function (band, bi) {
      var cy = top + bi * row + row / 2;
      var label = band.label.length > 38 ? band.label.slice(0, 35) + "…" : band.label;
      svg += '<text x="8" y="' + (cy + 3) + '" font-size="9" fill="#c9d1d9">' + esc(label) + "</text>";
      svg += '<line x1="' + left + '" y1="' + cy + '" x2="' + (width - right) + '" y2="' + cy + '" stroke="#30363d"/>';
      band.choices.forEach(function (p, pi) {
        var v = p[metric];
        if (v == null || !isFinite(v) || (logarithmic && v <= 0)) return;
        var leader = band.leader || band.choices[0];
        var isLeader = leader && leader.key === p.key;
        var py = cy + ((pi % 5) - 2) * 2.5;
        var color = famColor[p.family] || "#8b949e";
        var desc = shortLabel(p) + " — " + axisLabel + " " + valueLabel(v);
        const pointX = x(v).toFixed(1);
        const pointY = py.toFixed(1);
        svg += '<g class="xpt-control" data-pick="' + esc(p.key) + '" tabindex="0" role="button" aria-label="' + esc("Select " + desc) + '"><circle class="xpt-hit" cx="' + pointX + '" cy="' + pointY + '" r="40"/><circle class="xpt" cx="' + pointX + '" cy="' + pointY + '" r="' + (isLeader ? 5 : 3.8) + '" fill="' + color + '" fill-opacity="' + (isLeader ? 0.95 : 0.65) + '" stroke="' + (isLeader ? "#fff" : color) + '" stroke-width="' + (isLeader ? 1.5 : 0.8) + '"/><title>' + esc(desc + (isLeader ? " — band leader" : " — near-leader")) + "</title></g>";
      });
    });
    return '<div class="xpl-band-lane-wrap">' + svg + "</svg></div>";
  }
  function bandGroup(title, blurb, bands, laneMetric, axisLabel) {
    var rowsHtml = bands.map(function (b) {
      var leader = b.choices.length ? b.choices[0] : null;
      return "<tr><th scope=\\\"row\\\"><strong>" + esc(b.label) + '</strong><br><span class="muted">' + b.choices.length + " near-leading · " + esc(bandRangeLine(b.members)) + "</span></th><td>" +
        (b.choices.length ? b.choices.map(function (p) { return chipFor(p, leader); }).join(" ") : '<span class="muted">No model meets the window of this band.</span>') + "</td></tr>";
    }).join("");
    var chart = laneMetric ? bandLaneChart(title, laneMetric, axisLabel, bands) : "";
    return "<h3>" + esc(title) + '</h3><p class="muted">' + esc(blurb) + '</p>' + chart + '<div class="tblwrap"><table class="xpl-table"><caption>' + esc(title) + '</caption><tbody>' + rowsHtml + "</tbody></table></div>";
  }
  function renderBandComparisons() {
    var wrap = document.getElementById("xpl-bandgroups");
    if (!wrap) return;
    var rows = visPoints();
    var n = Math.max(2, Math.min(8, Number(S.bandCount) || 3));
    var scale = Number(S.bandScale) || 1;
    var scoreWin = 2 * scale, timeWin = 0.10 * scale, priceWin = 0.25 * scale;
    var costEdges = logEdges(rows.map(function (p) { return p.cost; }), n);
    var timeEdges = logEdges(rows.map(function (p) { return p.time; }), n);
    var sEdges = scoreEdges(rows, n);
    function groupBy(edges, get) {
      var out = [];
      for (var i = 0; i < edges.length - 1; i++) out.push([]);
      rows.forEach(function (p) {
        var v = get(p);
        if (v == null) return;
        var idx = bandIndex(v, edges);
        if (idx >= 0) out[idx].push(p);
      });
      return out;
    }
    function costLabel(i) { return (i === 0 ? "≥ " : "> ") + fmt$(costEdges[i]) + " · ≤ " + fmt$(costEdges[i + 1]); }
    function timeLabel(i) { return fmtT(timeEdges[i]) + " – " + fmtT(timeEdges[i + 1]); }
    function scoreLabel(i) { return fmtN(sEdges[i], 1) + (i === sEdges.length - 2 ? " – ≤ " : " – < ") + fmtN(sEdges[i + 1], 1); }
    var html = "";
    // 1. Smartest within each cost band
    if (costEdges.length > 1) {
      var g1 = groupBy(costEdges, function (p) { return p.cost; }).map(function (members, i) {
        var scored = members.filter(function (p) { return p.score != null; });
        var lead = scored.length ? Math.max.apply(null, scored.map(function (p) { return p.score; })) : null;
        var choices = scored.filter(function (p) { return lead != null && p.score >= lead - scoreWin; }).sort(function (a, b) { return cmpDesc(a.score, b.score); });
        return { label: "Cost band " + (i + 1) + " · " + costLabel(i), members: members, choices: choices };
      });
      var h1 = bandGroup("Smartest within each task-cost band", "Highest scores only within each listed task-cost range; near-leaders sit within " + fmtN(scoreWin, 2) + " points of the band's best score.", g1);
      // 5. Fastest within each cost band
      var g5 = groupBy(costEdges, function (p) { return p.cost; }).map(function (members, i) {
        var timed = members.filter(function (p) { return p.time != null; });
        var lead = timed.length ? Math.min.apply(null, timed.map(function (p) { return p.time; })) : null;
        var choices = timed.filter(function (p) { return lead != null && p.time <= lead * (1 + timeWin); }).sort(function (a, b) { return cmpAsc(a.time, b.time); });
        return { label: "Cost band " + (i + 1) + " · " + costLabel(i), members: members, choices: choices };
      });
      var h5 = bandGroup("Fastest within each task-cost band", "Disjoint task-cost tiers; near-leaders are within " + Math.round(timeWin * 100) + "% of the band's fastest time.", g5, "time", "Time / task (lower is faster)");
    }
    if (sEdges.length > 1) {
      // 2. Lowest cost within each score band
      var g2 = groupBy(sEdges, function (p) { return p.score; }).map(function (members, i) {
        var priced = members.filter(function (p) { return p.cost != null; });
        var lead = priced.length ? Math.min.apply(null, priced.map(function (p) { return p.cost; })) : null;
        var choices = priced.filter(function (p) { return lead != null && p.cost <= lead * (1 + priceWin); }).sort(function (a, b) { return cmpAsc(a.cost, b.cost); });
        return { label: scoreWord() + " band " + (i + 1) + " · " + scoreLabel(i), members: members, choices: choices };
      });
      var h2 = bandGroup("Lowest task cost within each score band", "Bands split the selected score range evenly; each model appears in one score band; near-price window " + Math.round(priceWin * 100) + "%.", g2);
      // 3. Fastest within each score band
      var g3 = groupBy(sEdges, function (p) { return p.score; }).map(function (members, i) {
        var timed = members.filter(function (p) { return p.time != null; });
        var lead = timed.length ? Math.min.apply(null, timed.map(function (p) { return p.time; })) : null;
        var choices = timed.filter(function (p) { return lead != null && p.time <= lead * (1 + timeWin); }).sort(function (a, b) { return cmpAsc(a.time, b.time); });
        return { label: scoreWord() + " band " + (i + 1) + " · " + scoreLabel(i), members: members, choices: choices };
      });
      var h3 = bandGroup("Fastest within each score band", "Within " + Math.round(timeWin * 100) + "% of the band's speed leader.", g3, "time", "Time / task (lower is faster)");
    }
    if (timeEdges.length > 1) {
      // 4. Smartest within each speed band (slowest first, as Site 1)
      var g4raw = groupBy(timeEdges, function (p) { return p.time; });
      var g4 = g4raw.map(function (members, i) {
        var scored = members.filter(function (p) { return p.score != null; });
        var lead = scored.length ? Math.max.apply(null, scored.map(function (p) { return p.score; })) : null;
        var choices = scored.filter(function (p) { return lead != null && p.score >= lead - scoreWin; }).sort(function (a, b) { return cmpDesc(a.score, b.score); });
        return { label: (i === g4raw.length - 1 ? "Slowest speed band" : i === 0 ? "Fastest speed band" : "Speed band " + (i + 1)) + " · " + timeLabel(i), members: members, choices: choices };
      }).reverse();
      var h4 = bandGroup("Smartest within each speed band", "Selected speed values split into ordered, non-overlapping bands from slowest to fastest; near-leaders within the score window.", g4, "score", scoreWord() + " (higher is better)");
      // 6. Lowest price within each speed target
      var timedAll = rows.filter(function (p) { return p.time != null; }).map(function (p) { return p.time; }).sort(function (a, b) { return a - b; });
      var tiers = [];
      for (var ti = 0; ti < costEdges.length - 1; ti++) tiers.push(ti);
      var g6 = [50, 75, 90, 100].map(function (q) {
        var thr = timedAll.length ? timedAll[Math.min(timedAll.length - 1, Math.ceil((q / 100) * timedAll.length) - 1)] : null;
        var cells = tiers.map(function (ci) {
          var qual = rows.filter(function (p) { return p.cost != null && p.time != null && thr != null && p.time <= thr && bandIndex(p.cost, costEdges) === ci; });
          if (!qual.length) return { label: "Tier " + (ci + 1) + " · " + costLabel(ci), members: [], choices: [], leader: null, empty: '<span class="muted">No model meets this target in this band.</span>' };
          var cheapest = qual.slice().sort(function (a, b) { return cmpAsc(a.cost, b.cost); })[0];
          var near = qual.filter(function (p) { return p.cost <= cheapest.cost * (1 + priceWin); }).sort(function (a, b) { return cmpAsc(a.cost, b.cost); });
          return { label: "Tier " + (ci + 1) + " · " + costLabel(ci), members: qual, choices: near, leader: cheapest };
        });
        return { label: q === 100 ? "Fastest available (100th percentile)" : "At least " + q + "th percentile", threshold: thr, cells: cells };
      });
      var targetRows = g6.map(function (target) {
        return "<tr><th scope=\\\"row\\\"><strong>" + target.label + '</strong><br><span class="muted">time ≤ ' + fmtT(target.threshold) + "</span></th>" + target.cells.map(function (cell) {
          return "<td>" + (cell.choices.length ? cell.choices.map(function (p) { return chipFor(p, cell.leader); }).join(" ") : cell.empty) + "</td>";
        }).join("") + "</tr>";
      }).join("");
      var g6Lanes = [];
      g6.forEach(function (target) {
        target.cells.forEach(function (cell) {
          g6Lanes.push({ label: target.label + " · " + cell.label, members: cell.members, choices: cell.choices, leader: cell.leader });
        });
      });
      var h6 = '<h3>Lowest price within each speed target</h3><p class="muted">Each speed target is split into the selected number of log-spaced task-cost bands; nearby options within ' + Math.round(priceWin * 100) + "% of the tier's cheapest.</p>" +
        '<div class="tblwrap"><table class="xpl-table"><caption>Lowest price within each speed target</caption><thead><tr><th scope="col">Speed target</th>' + tiers.map(function (ci) { return "<th scope=\\\"col\\\">Tier " + (ci + 1) + " · " + esc(costLabel(ci)) + "</th>"; }).join("") + "</tr></thead><tbody>" + targetRows + "</tbody></table></div>";
      h6 += bandLaneChart("Lowest price within each speed target", "cost", "USD / task (lower is cheaper)", g6Lanes);
    }
    html = (typeof h1 !== "undefined" ? h1 : "") + (typeof h2 !== "undefined" ? h2 : "") + (typeof h3 !== "undefined" ? h3 : "") + (typeof h4 !== "undefined" ? h4 : "") + (typeof h5 !== "undefined" ? h5 : "") + (typeof h6 !== "undefined" ? h6 : "");
    wrap.innerHTML = html || '<p class="muted">No banded comparisons can be built — the visible configurations lack the measures they need.</p>';
    wrap.querySelectorAll("[data-pick]").forEach(function (b) {
      b.addEventListener("click", function () { selectPoint(b.getAttribute("data-pick")); });
      b.addEventListener("keydown", function (ev) {
        if (ev.key === "Enter" || ev.key === " ") { ev.preventDefault(); selectPoint(b.getAttribute("data-pick")); }
      });
    });
  }
  function renderAaTable() {
    var tb = document.getElementById("xpl-rows");
    if (!tb) return;
    var rows = sortPoints(visPoints());
    var scoreHead = document.getElementById("xpl-scorehead");
    if (scoreHead) scoreHead.textContent = S.persp === "cai" ? "CAI score" : "Intelligence";
    document.getElementById("xpl-count").textContent =
      rows.length + " of " + POINTS.length + " ranked configurations · sorted by the " + S.xp + " perspective · dots are colored by model family; white-ringed dots sit on that chart's frontier, dimmed dots are beaten on its axes";
    tb.innerHTML = rows.map(function (p) {
      return '<tr data-sel="' + esc(p.key) + '" data-key="' + esc(p.key) + '"><td><strong>' + esc(p.family) + '</strong><br><span class="muted">' + esc(p.creator) + (p.qualStatus ? " · " + esc(p.qualStatus) : "") + '</span></td><td>' + esc(p.effort || "—") + '</td><td class="num">' + fmtN(p.score, 1) + '</td><td class="num xpl-costcol">' + fmt$(p.cost) + '</td><td class="num">' + fmtT(p.time) + '</td><td class="num">' + fmtT(p.ttft) + '</td><td class="num">' + fmtN(p.tps, 0) + '</td><td class="num xpl-costcol">' + fmtN(p.pointsPerDollar != null ? p.pointsPerDollar : p.scorePerDollar, 1) + '</td><td class="num">' + fmtN(p.pointsPerSecond != null ? p.pointsPerSecond : p.scorePerSecond, 3) + "</td></tr>";
    }).join("");
    tb.querySelectorAll("tr").forEach(function (tr) {
      tr.addEventListener("click", function () { selectPoint(tr.getAttribute("data-key")); });
    });
    markSelectedRow();
  }
  // --- task-assumptions projection (Site 2, in the Model Explorer) -------
  // ESTIMATED, NOT MEASURED: priced only from the token rates recorded
  // in the ainventory price event log (the catalog's rate bands). A
  // missing required rate yields an unknown cost, never a guess.
  // cost = turns x [uncached input + cache reads + cache writes +
  // (answer + thinking) output]; time = turns x (measured TTFT +
  // answer / measured throughput + tool overhead), borrowing the
  // family timing from either benchmark point set.
  function project(e) {
    return projectTask(e, S.pj, AA_POINTS, CAI_POINTS);
  }
  function renderModels() {
    var tb = document.getElementById("xpl-mrows");
    if (!tb) return;
    if (!CATALOG) return;
    var rows = sortCatalog(visCatalog());
    document.getElementById("xpl-mcount").textContent =
      rows.length + " of " + CATALOG.length + " catalog models · sorted by the " + S.xp + " perspective" +
      (S.xp === "api" ? " (effective $/task = AA list $/task ÷ the plan's multiplier, where the registry records one)" : "");
    tb.innerHTML = rows.map(function (e) {
      var ec = effCost(e);
      var proj = project(e);
      return '<tr data-sel="' + esc(e.id) + '" data-mid="' + esc(e.id) + '"><td><strong>' + esc(e.name) + '</strong><br><span class="muted"><code>' + esc(e.id) + "</code></span></td>" +
        "<td>" + esc(e.providerName) + (e.plan ? '<br><span class="badge src">' + esc(e.plan) + "</span>" : "") + (e.disabled ? ' <span class="badge stale">disabled</span>' : "") + '</td>' +
        '<td class="num">' + fmtCtx(e.contextLimit) + "</td>" +
        "<td>" + (e.variants.length ? esc(e.variants.join(", ")) : "—") + "</td>" +
        "<td>" + (e.routes.length ? e.routes.map(function (r) { return '<span class="badge open">' + esc(r) + "</span>"; }).join(" ") : '<span class="muted">—</span>') + "</td>" +
        '<td class="num">' + fmtN(e.aaScore, 1) + '</td><td class="num xpl-costcol">' + fmt$(e.aaCostPerTask) + '</td><td class="num xpl-costcol">' + (ec != null ? fmt$(ec) : "—") + '</td><td class="num">' + fmtT(e.aaTimePerTask) + '</td>' +
        '<td class="num xpl-costcol">' + (proj.cost != null ? "~" + fmt$(proj.cost) : "—") + '</td><td class="num">' + (proj.time != null ? "~" + fmtT(proj.time) : "—") + '</td><td class="num xpl-costcol">' + (e.aaScore != null && proj.cost ? fmtN(e.aaScore / proj.cost, 1) : "—") + "</td></tr>";
    }).join("");
    tb.querySelectorAll("tr").forEach(function (tr) {
      tr.addEventListener("click", function () { selectCatalog(tr.getAttribute("data-mid")); });
    });
    markSelectedRow();
  }
  function buildFilterOptions() {
    var creatorSel = document.getElementById("xf-creator");
    var creators = [];
    var seenC = {};
    POINTS.forEach(function (p) {
      var k = p.creator.toLowerCase();
      if (!seenC[k]) { seenC[k] = true; creators.push(p.creator); }
    });
    creators.sort();
    creatorSel.innerHTML = '<option value="">All creators</option>' + creators.map(function (c) { return '<option value="' + esc(c) + '"' + (S.creator === c ? " selected" : "") + ">" + esc(c) + "</option>"; }).join("");
    var benchSel = document.getElementById("xf-bench");
    var benches = [];
    POINTS.forEach(function (p) { if (p.benchmark && benches.indexOf(p.benchmark) < 0) benches.push(p.benchmark); });
    benchSel.innerHTML = '<option value="">All benchmarks</option>' + benches.map(function (b) { return '<option value="' + esc(b) + '"' + (S.bench === b ? " selected" : "") + ">" + esc(b) + "</option>"; }).join("");
    var effWrap = document.getElementById("xf-efforts");
    var efforts = [];
    POINTS.forEach(function (p) { var e = p.effort || ""; if (efforts.indexOf(e) < 0) efforts.push(e); });
    efforts.sort(function (a, b) { return eord(a) - eord(b); });
    effWrap.innerHTML = "";
    efforts.forEach(function (e) {
      var b = document.createElement("button");
      b.type = "button";
      b.className = "xchip" + (S.effortAny && !S.efforts[e] ? " off" : "");
      b.textContent = e || "(none)";
      b.addEventListener("click", function () {
        if (!S.effortAny) { S.effortAny = true; S.efforts = {}; S.efforts[e] = true; }
        else if (S.efforts[e]) { delete S.efforts[e]; if (!Object.keys(S.efforts).length) S.effortAny = false; }
        else S.efforts[e] = true;
        render();
      });
      effWrap.appendChild(b);
    });
    buildFamOptions();
  }
  function buildFamOptions() {
    var sel = document.getElementById("xf-fam");
    var label = document.getElementById("xf-fam-label");
    var opts = [];
    if (S.tab === "models" && CATALOG) {
      label.textContent = "Provider";
      var seen = {};
      CATALOG.forEach(function (e) { if (!seen[e.provider]) { seen[e.provider] = true; opts.push([e.provider, e.providerName]); } });
      opts.sort(function (a, b) { return a[1] < b[1] ? -1 : 1; });
      sel.innerHTML = '<option value="">All providers</option>' + opts.map(function (o) { return '<option value="' + esc(o[0]) + '"' + (S.fam === o[0] ? " selected" : "") + ">" + esc(o[1]) + "</option>"; }).join("");
    } else {
      label.textContent = "Family";
      sel.innerHTML = '<option value="">All families</option>' + famList.slice().sort().map(function (f) { return '<option value="' + esc(f) + '"' + (S.fam === f ? " selected" : "") + ">" + esc(f) + "</option>"; }).join("");
    }
  }
  function applyTab() {
    document.querySelectorAll("[data-xtab]").forEach(function (b) {
      b.classList.toggle("active", b.getAttribute("data-xtab") === S.tab);
    });
    document.getElementById("xtab-aa").hidden = S.tab !== "aa";
    document.getElementById("xtab-models").hidden = S.tab !== "models";
    document.querySelectorAll("[data-tabaa]").forEach(function (el) { el.hidden = S.tab !== "aa"; });
    document.querySelectorAll("[data-tabmodels]").forEach(function (el) { el.hidden = S.tab !== "models"; });
    buildFamOptions();
  }
  function render() {
    document.body.classList.toggle("xp-plan", S.xp === "plan");
    document.getElementById("xp-note").textContent = XP_NOTES[S.xp];
    document.querySelectorAll("[data-xp]").forEach(function (b) {
      b.classList.toggle("active", b.getAttribute("data-xp") === S.xp);
    });
    document.querySelectorAll("[data-persp]").forEach(function (b) {
      b.classList.toggle("active", b.getAttribute("data-persp") === S.persp);
    });
    var pnote = document.getElementById("xpl-persp-note");
    if (pnote) {
      pnote.textContent = S.persp === "cai"
        ? "Coding agents perspective — the Coding Agent Index observations (model + effort + agent harness): score is the CAI score, cost the published cost per coding task, time the average agent wall time per task. Kept separate from the Intelligence Index; the two are never converted."
        : "Intelligence & decode perspective — the AA Intelligence Index rankings: score is the Intelligence Index, cost and time the import's per-task figures (time excludes TTFT).";
    }
    applyTab();
    if (S.tab === "aa") { renderLegend(); renderTray(); drawCharts(); renderAaTable(); renderPareto(); renderRecs(); renderBandComparisons(); }
    else renderModels();
    enhanceCharts();
  }
  document.querySelectorAll("[data-xtab]").forEach(function (b) {
    b.addEventListener("click", function () {
      S.tab = b.getAttribute("data-xtab");
      S.fam = "";
      history.replaceState(null, "", S.tab === "models" ? "#models" : "#aa");
      render();
    });
  });
  document.querySelectorAll("[data-xp]").forEach(function (b) {
    b.addEventListener("click", function () { S.xp = b.getAttribute("data-xp"); render(); });
  });
  function setPerspective(persp) {
    if (S.persp === persp) return;
    S.persp = persp;
    POINTS = persp === "cai" ? CAI_POINTS : AA_POINTS;
    S.creator = ""; S.fam = ""; S.bench = ""; S.effortAny = false; S.efforts = {};
    S.off = {}; S.hidden = []; S.sel = null; S.baseline = "";
    showDetail(null);
    buildFilterOptions();
    buildBaselineOptions();
    render();
  }
  document.querySelectorAll("[data-persp]").forEach(function (b) {
    b.addEventListener("click", function () { setPerspective(b.getAttribute("data-persp")); });
  });
  var pfCutEl = document.getElementById("xf-pfcut");
  if (pfCutEl) pfCutEl.addEventListener("change", function (ev) { S.pfCut = ev.target.value; renderPareto(); });
  var bandsEl = document.getElementById("xf-bands");
  if (bandsEl) bandsEl.addEventListener("change", function (ev) { S.bandCount = Number(ev.target.value); renderRecs(); renderBandComparisons(); enhanceCharts(); });
  var bandScaleEl = document.getElementById("xf-bandscale");
  if (bandScaleEl) bandScaleEl.addEventListener("change", function (ev) { S.bandScale = Number(ev.target.value); renderBandComparisons(); });
  function syncRecInputs() {
    document.querySelectorAll("[data-rec]").forEach(function (el) {
      var f = el.getAttribute("data-rec");
      if (S.rec[f] != null) el.value = S.rec[f];
    });
    var sp = document.getElementById("rec-speedpriority-val");
    if (sp) sp.textContent = S.rec.speedPriority + "% speed";
  }
  document.querySelectorAll("[data-rec]").forEach(function (el) {
    el.addEventListener("input", function () {
      S.rec[el.getAttribute("data-rec")] = el.value;
      syncRecInputs();
      renderRecs();
    });
    el.addEventListener("change", function () {
      S.rec[el.getAttribute("data-rec")] = el.value;
      renderRecs();
    });
  });
  var REC_PRESETS = {
    fast: { allow: 5, speedPriority: 95, priceAllow: 200, slowAllow: 1.5, altWindow: 2, diversity: "lab", dominance: "lab", perBand: 3 },
    balanced: { allow: 5, speedPriority: 50, priceAllow: 500, slowAllow: 2, altWindow: 2, diversity: "lab", dominance: "lab", perBand: 3 },
    diversity: { allow: 7, speedPriority: 60, priceAllow: 500, slowAllow: 2.5, altWindow: 3, diversity: "labsfirst", dominance: "lab", perBand: 4 },
    budget: { allow: 5, speedPriority: 15, priceAllow: 100, slowAllow: 3, altWindow: 2, diversity: "lab", dominance: "all", perBand: 3 },
    reset: { allow: 5, speedPriority: 85, priceAllow: 500, slowAllow: 2, altWindow: 2, diversity: "lab", dominance: "lab", perBand: 3, maxCost: "", maxSec: "" },
  };
  document.querySelectorAll("[data-recpreset]").forEach(function (b) {
    b.addEventListener("click", function () {
      var preset = REC_PRESETS[b.getAttribute("data-recpreset")];
      if (!preset) return;
      Object.keys(preset).forEach(function (k) { S.rec[k] = preset[k]; });
      syncRecInputs();
      renderRecs();
    });
  });
  function syncPjInputs() {
    document.querySelectorAll("[data-pj]").forEach(function (el) {
      var f = el.getAttribute("data-pj");
      if (S.pj[f] != null) el.value = S.pj[f];
    });
    var cv = document.getElementById("pj-cacheread-val");
    if (cv) cv.textContent = S.pj.cacheReadPct + "%";
  }
  document.querySelectorAll("[data-pj]").forEach(function (el) {
    var on = function () { S.pj[el.getAttribute("data-pj")] = el.value; syncPjInputs(); if (S.tab === "models") renderModels(); };
    el.addEventListener("input", on);
    el.addEventListener("change", on);
  });
  var PJ_PRESETS = {
    small: { turns: 2, inputTokens: 8000, answerTokens: 1200, thinkingTokens: 2000, cacheReadPct: 60, writesPct: 0, overheadSec: 8 },
    feature: { turns: 4, inputTokens: 25000, answerTokens: 3000, thinkingTokens: 6000, cacheReadPct: 50, writesPct: 5, overheadSec: 10 },
    repo: { turns: 8, inputTokens: 60000, answerTokens: 4000, thinkingTokens: 8000, cacheReadPct: 70, writesPct: 5, overheadSec: 15 },
  };
  document.querySelectorAll("[data-pjpreset]").forEach(function (b) {
    b.addEventListener("click", function () {
      var preset = PJ_PRESETS[b.getAttribute("data-pjpreset")];
      if (!preset) return;
      Object.keys(preset).forEach(function (k) { S.pj[k] = preset[k]; });
      syncPjInputs();
      if (S.tab === "models") renderModels();
    });
  });
  var recExport = document.getElementById("xpl-rec-export");
  if (recExport) {
    recExport.addEventListener("click", function () {
      var out = computeRecs();
      var lines = ["band,rank,model,effort,lab,harness,score,cost_per_task,time_per_task_s,tradeoff_multiple"];
      out.bands.forEach(function (band) {
        var bestFactor = band.picks.length ? band.picks[0].factor : 1;
        band.picks.forEach(function (r, ri) {
          lines.push([band.label, ri + 1, r.p.family, r.p.effort || "", r.p.creator, r.p.harness || "", r.p.score, r.p.cost, r.p.time, (r.factor / bestFactor).toFixed(2)].map(function (v) { return '"' + String(v == null ? "" : v).replace(/"/g, '""') + '"'; }).join(","));
        });
      });
      downloadFile("cost-explorer-shortlist.csv", lines.join("\\n"), "text/csv");
    });
  }
  document.getElementById("xf-q").addEventListener("input", function (ev) { S.q = ev.target.value.trim().toLowerCase(); render(); });
  document.getElementById("xf-creator").addEventListener("change", function (ev) { S.creator = ev.target.value; render(); });
  document.getElementById("xf-fam").addEventListener("change", function (ev) { S.fam = ev.target.value; render(); });
  document.getElementById("xf-bench").addEventListener("change", function (ev) { S.bench = ev.target.value; render(); });
  document.getElementById("xf-elig").addEventListener("change", function (ev) { S.elig = ev.target.checked; render(); });
  document.getElementById("xf-pooled").addEventListener("change", function (ev) { S.pooledOnly = ev.target.checked; render(); });
  document.getElementById("xf-measured").addEventListener("change", function (ev) { S.measuredOnly = ev.target.checked; render(); });
  document.getElementById("xf-nodis").addEventListener("change", function (ev) { S.hideDisabled = ev.target.checked; render(); });
  document.querySelectorAll("[data-sizeby]").forEach(function (cb) {
    cb.addEventListener("change", function () { S.sizeBy[cb.getAttribute("data-sizeby")] = cb.checked; drawCharts(); });
  });
  document.querySelectorAll("[data-dl]").forEach(function (b) {
    b.addEventListener("click", function () { downloadPng(b.getAttribute("data-dl")); });
  });
  document.querySelectorAll("[data-tmode]").forEach(function (b) {
    b.addEventListener("click", function () { S.tasksMode = b.getAttribute("data-tmode"); drawCharts(); });
  });
  var baselineSel = document.getElementById("xf-baseline");
  if (baselineSel) baselineSel.addEventListener("change", function (ev) { S.baseline = ev.target.value; drawCharts(); });
  // --- inequality filter rows ---------------------------------------------
  var INEQ_METRICS = [["score", "Score"], ["cost", "Task cost"], ["time", "Selected speed (time / task)"], ["tps", "Output speed (tok/s)"], ["gapScore", "Pareto gap: score gap (vs cost frontier)"], ["gapCost", "Pareto gap: price premium (vs score)"], ["gapSpeed", "Pareto gap: speed gap (vs time frontier)"]];
  function renderIneqConds() {
    var wrap = document.getElementById("xf-ineq-conds");
    if (!wrap) return;
    wrap.innerHTML = S.ineq.conds.map(function (c, i) {
      var metricOpts = INEQ_METRICS.map(function (m) { return '<option value="' + m[0] + '"' + (c.metric === m[0] ? " selected" : "") + ">" + m[1] + "</option>"; }).join("");
      var opOpts = ["<", "<=", "=", ">=", ">"].map(function (o) { return '<option value="' + o + '"' + (c.op === o ? " selected" : "") + ">" + o + "</option>"; }).join("");
      return '<span class="xpl-ineq-row"><label>Condition ' + (i + 1) + ' <select data-ineq-metric="' + i + '">' + metricOpts + '</select></label> <select data-ineq-op="' + i + '" aria-label="Operator">' + opOpts + '</select> <input type="number" step="any" data-ineq-value="' + i + '" value="' + esc(c.value) + '" style="width:7em"> <button type="button" class="xpl-ctl-btn" data-ineq-remove="' + i + '">Remove</button></span>';
    }).join("");
    wrap.querySelectorAll("[data-ineq-metric]").forEach(function (el) { el.addEventListener("change", function () { S.ineq.conds[Number(el.getAttribute("data-ineq-metric"))].metric = el.value; render(); }); });
    wrap.querySelectorAll("[data-ineq-op]").forEach(function (el) { el.addEventListener("change", function () { S.ineq.conds[Number(el.getAttribute("data-ineq-op"))].op = el.value; render(); }); });
    wrap.querySelectorAll("[data-ineq-value]").forEach(function (el) { el.addEventListener("input", function () { S.ineq.conds[Number(el.getAttribute("data-ineq-value"))].value = el.value; render(); }); });
    wrap.querySelectorAll("[data-ineq-remove]").forEach(function (el) { el.addEventListener("click", function () { S.ineq.conds.splice(Number(el.getAttribute("data-ineq-remove")), 1); renderIneqConds(); render(); }); });
  }
  var ineqModeEl = document.getElementById("xf-ineq-mode");
  if (ineqModeEl) ineqModeEl.addEventListener("change", function (ev) { S.ineq.mode = ev.target.value; render(); });
  var ineqCombineEl = document.getElementById("xf-ineq-combine");
  if (ineqCombineEl) ineqCombineEl.addEventListener("change", function (ev) { S.ineq.combine = ev.target.value; render(); });
  var ineqAddEl = document.getElementById("xf-ineq-add");
  if (ineqAddEl) ineqAddEl.addEventListener("click", function () { S.ineq.conds.push({ metric: "score", op: ">=", value: "" }); renderIneqConds(); });
  // --- saved setups + downloads ---------------------------------------------
  function captureSetup() {
    return JSON.parse(JSON.stringify({
      tab: S.tab, persp: S.persp, xp: S.xp, q: S.q, creator: S.creator, fam: S.fam, bench: S.bench,
      effortAny: S.effortAny, efforts: S.efforts, off: S.off, elig: S.elig, pooledOnly: S.pooledOnly,
      measuredOnly: S.measuredOnly, hideDisabled: S.hideDisabled, hidden: S.hidden, sizeBy: S.sizeBy,
      tasksMode: S.tasksMode, baseline: S.baseline, pfCut: S.pfCut, bandCount: S.bandCount,
      bandScale: S.bandScale, rec: S.rec, pj: S.pj, ineq: S.ineq,
    }));
  }
  function syncStaticControls() {
    var q = document.getElementById("xf-q"); if (q) q.value = S.q;
    var elig = document.getElementById("xf-elig"); if (elig) elig.checked = S.elig;
    var pooled = document.getElementById("xf-pooled"); if (pooled) pooled.checked = S.pooledOnly;
    var meas = document.getElementById("xf-measured"); if (meas) meas.checked = S.measuredOnly;
    var nodis = document.getElementById("xf-nodis"); if (nodis) nodis.checked = S.hideDisabled;
    var pf = document.getElementById("xf-pfcut"); if (pf) pf.value = S.pfCut;
    var bd = document.getElementById("xf-bands"); if (bd) bd.value = String(S.bandCount);
    var bs = document.getElementById("xf-bandscale"); if (bs) bs.value = String(S.bandScale);
    if (ineqModeEl) ineqModeEl.value = S.ineq.mode;
    if (ineqCombineEl) ineqCombineEl.value = S.ineq.combine;
    document.querySelectorAll("[data-tmode]").forEach(function (b) { b.classList.toggle("active", b.getAttribute("data-tmode") === S.tasksMode); });
    document.querySelectorAll("[data-sizeby]").forEach(function (cb) { cb.checked = !!S.sizeBy[cb.getAttribute("data-sizeby")]; });
  }
  function applySetup(st) {
    if (!st || typeof st !== "object") return;
    ["tab", "persp", "xp", "q", "creator", "fam", "bench", "effortAny", "elig", "pooledOnly", "measuredOnly", "hideDisabled", "tasksMode", "baseline", "pfCut", "bandCount", "bandScale"].forEach(function (k) { if (st[k] !== undefined) S[k] = st[k]; });
    ["efforts", "off", "hidden", "sizeBy", "rec", "pj", "ineq"].forEach(function (k) { if (st[k] !== undefined) S[k] = st[k]; });
    POINTS = S.persp === "cai" ? CAI_POINTS : AA_POINTS;
    S.sel = null;
    showDetail(null);
    buildFilterOptions();
    buildBaselineOptions();
    syncRecInputs();
    syncPjInputs();
    renderIneqConds();
    syncStaticControls();
    render();
  }
  function loadSetups() {
    // #2851: the store is null-prototype — on a plain object a setup
    // named "__proto__" assigns the prototype instead of becoming an
    // entry (silently never saved), and an imported "__proto__" entry
    // mutates the live store's prototype. Copy the parsed own keys
    // into Object.create(null) so every name is an ordinary entry.
    var out = Object.create(null);
    try {
      var parsed = JSON.parse(localStorage.getItem("xpl-setups-v1") || "{}");
      if (parsed && typeof parsed === "object") Object.keys(parsed).forEach(function (k) { out[k] = parsed[k]; });
    } catch (e) { /* unreadable storage: empty store */ }
    return out;
  }
  function storeSetups(obj) {
    try { localStorage.setItem("xpl-setups-v1", JSON.stringify(obj)); } catch (e) { /* storage unavailable: setups stay session-only */ }
  }
  function refreshSetupList(selected) {
    var sel = document.getElementById("xf-setup-list");
    if (!sel) return;
    var names = Object.keys(loadSetups()).sort();
    sel.innerHTML = names.length ? names.map(function (n) { return '<option value="' + esc(n) + '"' + (n === selected ? " selected" : "") + ">" + esc(n) + "</option>"; }).join("") : '<option value="">(none saved)</option>';
  }
  var setupSaveEl = document.getElementById("xf-setup-save");
  if (setupSaveEl) setupSaveEl.addEventListener("click", function () {
    var nameEl = document.getElementById("xf-setup-name");
    var name = nameEl && nameEl.value.trim() ? nameEl.value.trim() : "Setup " + (Object.keys(loadSetups()).length + 1);
    var all = loadSetups();
    all[name] = captureSetup();
    storeSetups(all);
    refreshSetupList(name);
  });
  var setupLoadEl = document.getElementById("xf-setup-load");
  if (setupLoadEl) setupLoadEl.addEventListener("click", function () {
    var sel = document.getElementById("xf-setup-list");
    if (sel && sel.value) applySetup(loadSetups()[sel.value]);
  });
  var setupExportEl = document.getElementById("xf-setup-export");
  if (setupExportEl) setupExportEl.addEventListener("click", function () { downloadFile("cost-explorer-setups.json", JSON.stringify(loadSetups(), null, 2), "application/json"); });
  var setupImportEl = document.getElementById("xf-setup-import");
  if (setupImportEl) setupImportEl.addEventListener("change", function () {
    var file = setupImportEl.files && setupImportEl.files[0];
    if (!file) return;
    var reader = new FileReader();
    reader.onload = function () {
      try {
        var parsed = JSON.parse(String(reader.result));
        if (parsed && typeof parsed === "object") {
          var all = loadSetups();
          Object.keys(parsed).forEach(function (k) { all[k] = parsed[k]; });
          storeSetups(all);
          refreshSetupList();
        }
      } catch (e) { /* unreadable file: nothing imported */ }
    };
    reader.readAsText(file);
  });
  var setupResetEl = document.getElementById("xf-setup-reset");
  if (setupResetEl) setupResetEl.addEventListener("click", function () {
    applySetup({
      tab: "aa", persp: "aa", xp: "api", q: "", creator: "", fam: "", bench: "", effortAny: false, efforts: {}, off: {},
      elig: false, pooledOnly: false, measuredOnly: false, hideDisabled: false, hidden: [], sizeBy: {}, tasksMode: "per",
      baseline: "", pfCut: "50", bandCount: 3, bandScale: 1,
      rec: { allow: 5, speedPriority: 85, priceAllow: 500, slowAllow: 2, altWindow: 2, diversity: "lab", dominance: "lab", perBand: 3, maxCost: "", maxSec: "" },
      pj: { turns: 3, inputTokens: 20000, answerTokens: 2000, thinkingTokens: 4000, cacheReadPct: 50, writesPct: 0, overheadSec: 10, window: "peak" },
      ineq: { mode: "off", combine: "and", conds: [{ metric: "score", op: ">=", value: "" }, { metric: "cost", op: "<=", value: "" }, { metric: "time", op: "<=", value: "" }] },
    });
  });
  function csvCell(v) { return '"' + String(v == null ? "" : v).replace(/"/g, '""') + '"'; }
  var dlJsonEl = document.getElementById("xf-dl-json");
  if (dlJsonEl) dlJsonEl.addEventListener("click", function () { downloadFile("cost-explorer-data.json", JSON.stringify(DATA, null, 2), "application/json"); });
  var dlCsvEl = document.getElementById("xf-dl-csv");
  if (dlCsvEl) dlCsvEl.addEventListener("click", function () {
    var lines = [];
    if (S.tab === "models") {
      lines.push(["model_id", "name", "provider", "plan", "context", "aa_score", "aa_cost_per_task", "proj_cost_per_task", "proj_time_per_task_s"].join(","));
      sortCatalog(visCatalog()).forEach(function (e) {
        var proj = project(e);
        lines.push([e.id, e.name, e.providerName, e.plan || "", e.contextLimit, e.aaScore, e.aaCostPerTask, proj.cost, proj.time].map(csvCell).join(","));
      });
      downloadFile("cost-explorer-models-visible.csv", lines.join("\\n"), "text/csv");
    } else {
      lines.push(["family", "effort", "creator", "benchmark", "harness", "score", "cost_per_task", "time_per_task_s", "ttft_s", "tokens_per_second", "tokens_per_task"].join(","));
      sortPoints(visPoints()).forEach(function (p) {
        lines.push([p.family, p.effort || "", p.creator, p.benchmark || "", p.harness || "", p.score, p.cost, p.time, p.ttft, p.tps, p.tokensPerTask].map(csvCell).join(","));
      });
      downloadFile("cost-explorer-configurations-visible.csv", lines.join("\\n"), "text/csv");
    }
  });
  renderIneqConds();
  refreshSetupList();
  buildFilterOptions();
  buildBaselineOptions();
  render();
})();
</script>`;

function fmtCost(v: number | null): string {
  return v == null ? "—" : `$${Number(v.toFixed(4))}`;
}

function fmtSec(v: number | null): string {
  if (v == null) return "—";
  if (v < 60) return `${Number(v.toFixed(1))}s`;
  return `${Math.floor(v / 60)}m ${Math.round(v % 60)}s`;
}

function fmtTimed(v: number | null): string {
  return v == null ? '<span class="muted">not measured</span>' : fmtSec(v);
}

/** "openai|gpt-6-luna|max|reasoning|fallback-na" -> "gpt-6-luna · max". */
function variantLabel(modelVariantKey: string): string {
  const parts = modelVariantKey.split("|");
  return parts.length >= 3 ? `${parts[1]} · ${parts[2]}` : modelVariantKey;
}

function aaTimingsTable(testRuns: AaTestRun[]): string {
  const rows = testRuns
    .map(
      (r) =>
        `<tr><td><strong>${esc(variantLabel(r.modelVariantKey))}</strong><br><span class="muted">${esc(r.snapshotDate ?? "no snapshot date")}</span></td><td>${r.intelligence ?? "—"}</td><td>${fmtCost(r.costPerTaskUsd)}</td><td>${fmtTimed(r.ttftSeconds)}</td><td>${fmtTimed(r.firstAnswerSeconds)}</td><td>${fmtTimed(r.e2e500Seconds)}</td><td>${fmtTimed(r.timePerTaskSeconds)}</td><td>${r.outputSpeedTps ?? "—"}</td></tr>`,
    )
    .join("");
  return `<div class="tblwrap"><table class="table table-zebra"><caption>Model configurations — intelligence and cost</caption><tr><th scope="col">Configuration</th><th scope="col">Intelligence</th><th scope="col">$/task</th><th scope="col">TTFT</th><th scope="col">First answer</th><th scope="col">E2E (500 tok)</th><th scope="col">Time / task</th><th scope="col">Speed (t/s)</th></tr>${rows || '<tr><td colspan="8" class="muted">none</td></tr>'}</table></div>`;
}

function aaConfigsTable(configs: AaConfig[]): string {
  const rows = configs
    .map(
      (c) =>
        `<tr><td><strong>${esc(c.modelName)}</strong></td><td>${esc(c.effort ?? "—")}</td><td>${c.intelligence ?? "—"}</td><td>${fmtCost(c.costPerTaskUsd)}</td><td>${c.speedTps ?? "—"}</td></tr>`,
    )
    .join("");
  return `<div class="tblwrap"><table class="table table-zebra"><caption>Models — effort, intelligence, cost, and speed</caption><tr><th scope="col">Model</th><th scope="col">Effort (AA label)</th><th scope="col">Intelligence</th><th scope="col">$/task</th><th scope="col">Speed (t/s)</th></tr>${rows || '<tr><td colspan="5" class="muted">none</td></tr>'}</table></div>`;
}

function aaCaiTable(rowsIn: AaCaiRow[]): string {
  const pct = (v: number | null): string => (v == null ? "—" : `${Number((v * 100).toFixed(1))}%`);
  const rows = rowsIn
    .map(
      (r) =>
        `<tr><td><strong>${esc(r.family)}</strong><br><span class="muted">${esc(r.creator)}${r.snapshotDate ? ` · ${esc(r.snapshotDate)}` : ""}</span></td><td>${esc(r.effort ?? "—")}</td><td>${esc(r.harness ?? "—")}</td><td>${esc(r.version ?? "—")}</td><td>${r.score ?? "—"}</td><td>${pct(r.deepSwe)}</td><td>${pct(r.terminalBench)}</td><td>${pct(r.sweAtlas)}</td><td>${fmtCost(r.cost)}</td><td>${fmtTimed(r.wallSec)}</td><td>${r.tokensPerTask != null ? Math.round(r.tokensPerTask).toLocaleString("en-US") : "—"}</td></tr>`,
    )
    .join("");
  return `<h2>Coding Agent Index observations</h2>
<p class="muted">The import's Coding Agent Index rows — a model plus its agent harness and settings, checked as published. Kept strictly separate from the AA Intelligence Index rows above: no score is converted between the indexes. "Not published" costs stay blank rather than estimated. These rows power the explorer's Coding agents perspective.</p>
<div class="tblwrap"><table class="table table-zebra"><caption>Model benchmark scores</caption><tr><th scope="col">Model</th><th scope="col">Effort</th><th scope="col">Harness</th><th scope="col">Version</th><th scope="col">CAI score</th><th scope="col">DeepSWE v1.1</th><th scope="col">Terminal-Bench 4.0</th><th scope="col">SWE-Atlas-QnA</th><th scope="col">$/task</th><th scope="col">Agent wall time / task</th><th scope="col">Tokens / task</th></tr>${rows || '<tr><td colspan="11" class="muted">none</td></tr>'}</table></div>`;
}

function aaNameList(names: string[]): string {
  return names.length
    ? `<div class="strip">${names.map((n) => `<span class="badge src">${esc(n)}</span>`).join("")}</div>`
    : '<p class="muted">none</p>';
}

/** JSON for a <script type="application/json"> block: `<` is the only
 * character that can break out of it, so it crosses as its escape. */
function jsonForScript(json: string): string {
  return json.replace(/</g, "\\u003c");
}

const AA_COLLECTOR_CADENCE_MS = 5 * 60_000;

function aaFreshnessChips(detail: ProvenanceDetail | undefined, section: AaFeedSection | null): string {
  if (!detail) return "";
  const aaFacet = detail.aa;
  const olderThanOneCycle =
    aaFacet.state === "live" && aaFacet.ageMs !== undefined && aaFacet.ageMs > AA_COLLECTOR_CADENCE_MS;
  const retainedAfterFailure = section?.status === "ok" && section.stale === true;
  const chipDetail: ProvenanceDetail =
    olderThanOneCycle || retainedAfterFailure ? { ...detail, aa: { ...aaFacet, state: "stale" } } : detail;
  return freshnessChips(chipDetail, ["aa"]);
}

export function renderAa(section: AaFeedSection | null, provenance: Provenance, detail?: ProvenanceDetail): string {
  if (!section) {
    return layout(
      "Cost Explorer",
      `<h1>Cost Explorer</h1><div class="banner">The AA data has not reached the dashboard yet — the fleet-feed collector reads it from Supabase (ainventory), and no AA section is in the current feed. The data of record is the ainventory import; this page only renders it.</div>`,
      provenance,
      "/aa",
      "",
      aaFreshnessChips(detail, null),
    );
  }
  if (section.status === "absent") {
    return layout(
      "Cost Explorer",
      `<h1>Cost Explorer</h1><div class="banner">The collector could not build the AA section on its last run (${fmtDate(section.generatedAt)}): ${esc(section.reason)}. No successful collection exists yet to fall back to — the page fills in on the first successful collection.</div>`,
      provenance,
      "/aa",
      "",
      aaFreshnessChips(detail, section),
    );
  }
  const s: AaSection = section;
  const summary = summarizeAa(s);
  const sourcesHtml = s.sources
    .map(
      (src) =>
        `<li><code>${esc(src.name)}</code> (${esc(src.kind)}) — imported ${src.importedAt ? fmtDate(src.importedAt) : "—"} · file modified ${src.driveModifiedAt ? fmtDate(src.driveModifiedAt) : "—"}</li>`,
    )
    .join("");
  const cov = s.coverage;
  const coverageHtml = cov
    ? `<h2>Coverage — measured but not pooled</h2>
<p class="muted">The collector compares the AA-measured model names against the repo's <code>.opencode/opencode.jsonc</code>: the provider catalogs and every route pool's leaf targets (nested routes expanded). Matching is best-effort — normalization plus a small alias map in <code>src/aa.ts</code> — so a miss means "not matched", not "does not exist".</p>
<h3>In a catalog, in no pool (${cov.catalogNotPooledMeasured.length})</h3>
<p class="muted">Measured and reachable today, but no route sends them work — the actionable list.</p>${aaNameList(cov.catalogNotPooledMeasured)}
<h3>Measured, in no catalog (${cov.measuredNotInCatalog.length})</h3>${aaNameList(cov.measuredNotInCatalog)}
<details><summary>Catalog models with no AA measurement and no pool seat (${cov.catalogNotPooledUnmeasured.length})</summary>
<p class="muted">Catalog ids verbatim from the config — this is where unmeasured newcomers (e.g. <code>commandcode/reflection/beam:free</code>, the Reflection Beam seat) sit until the import measures them.</p>${aaNameList(cov.catalogNotPooledUnmeasured)}</details>`
    : `<h2>Coverage — measured but not pooled</h2><p class="muted">Coverage was not computed for this section — the collector could not read <code>.opencode/opencode.jsonc</code> on that run.</p>`;
  const catalog = s.catalog;
  const feedAge = detail?.aa.ageMs === undefined ? "unavailable" : ageMsText(detail.aa.ageMs);
  const aaTabAge = detail ? `<p class="muted" data-aa-age>AA data age: ${esc(feedAge)} (fleet-feed age).</p>` : "";
  const catalogTabAge = detail
    ? catalog
      ? `<p class="muted" data-catalog-age>Model catalog age: ${esc(feedAge)} (fleet-feed age; catalog and AA are read in the same collector run; no separate catalog timestamp is carried).</p>`
      : `<p class="muted" data-catalog-age>Model catalog age: unavailable — no catalog in this feed (fleet-feed age: ${esc(feedAge)}).</p>`
    : "";
  const modelCards = catalog
    ? `<div class="board">
<div class="card"><strong>${catalog.length} models</strong> in the fleet's provider catalogs<br><span class="muted">from <code>.opencode/opencode.jsonc</code>, read by the same collector run as the AA data</span></div>
<div class="card"><strong>${new Set(catalog.map((e) => e.provider)).size} providers</strong> · ${catalog.filter((e) => e.disabled).length} models behind disabled providers<br><span class="muted">Disabled means catalogued but not callable — the config disables the door, the proxy layer never does</span></div>
<div class="card"><strong>${catalog.filter((e) => e.routes.length > 0).length} pooled</strong> · ${catalog.filter((e) => e.aaScore !== null).length} AA-measured<br><span class="muted">Pooled = at least one route sends it work; measured = the import has an intelligence score for its name</span></div>
</div>`
    : `<div class="banner">The collector has not carried the fleet catalog yet — it is built from the same <code>.opencode/opencode.jsonc</code> read as the coverage section and appears once a collector build that knows it has run. No catalog is shown rather than a stale one.</div>`;
  const explorerJson = jsonForScript(
    JSON.stringify({ points: aaPoints(s), caiPoints: s.cai ? aaCaiPoints(s.cai) : [], catalog: catalog ?? null }),
  );
  const body = `<h1>Cost Explorer</h1>
${s.stale ? `<div class="banner">The latest collection failed (${esc(s.staleReason ?? "unknown error")}) — showing the last good data, collected ${fmtDate(s.generatedAt)}. It refreshes automatically on the next successful collection.</div>` : ""}
<p class="muted">${s.rankings.length} ranked configurations · ${s.configs.length} AA Intelligence Index configurations · ${s.testRuns.length} timing rows${s.cai ? ` · ${s.cai.length} Coding Agent Index observations` : ""}${catalog ? ` · ${catalog.length} catalog models` : ""} · collected ${fmtDate(s.generatedAt)} from ainventory batch <code>${esc(s.batch.slice(0, 8))}…</code>. Machine-first: <a href="/api/aa.json">/api/aa.json</a>.</p>
<div class="card"><strong>Provenance</strong><br>
<span class="muted">These are the import snapshot's figures, not a live Artificial Analysis pull.${summary.snapshotVintage ? ` Test_Runs snapshots in this batch: ${esc(summary.snapshotVintage.min)} – ${esc(summary.snapshotVintage.max)}.` : ""} A new import (coding-plan-watch) flips the batch; the feed collector picks it up within minutes. The explorer renders from the feed alone — when the data pipeline lands new models (Terra, Beam), they appear here with no page change.</span>
<ul>${sourcesHtml || '<li class="muted">source files not recorded for this batch</li>'}</ul></div>
<div class="xtabs" role="tablist"><button type="button" class="xtab" data-xtab="aa">AA Explorer</button><button type="button" class="xtab" data-xtab="models">Model Explorer</button></div>
<div class="card xpl-filters" id="xpl-filters">
<input type="search" id="xf-q" placeholder="Search models, families, providers…" aria-label="Search">
<label data-tabaa>Creator <select id="xf-creator"></select></label>
<label><span id="xf-fam-label">Family</span> <select id="xf-fam"></select></label>
<label data-tabaa>Benchmark <select id="xf-bench"></select></label>
<span class="xp-seg" role="group" aria-label="Efficiency perspective"><button type="button" data-xp="plan">Plan</button><button type="button" data-xp="api">API</button><button type="button" data-xp="pertask">Per-task</button></span>
<span class="xp-seg" role="group" aria-label="Benchmark perspective" data-tabaa><button type="button" data-persp="aa">Intelligence &amp; decode</button><button type="button" data-persp="cai">Coding agents</button></span>
<p class="muted" id="xpl-persp-note" data-tabaa style="flex-basis:100%;margin:2px 0 0"></p>
<div data-tabaa style="flex-basis:100%"><span class="muted">Efforts:</span> <span id="xf-efforts"></span> <label style="margin-left:10px"><input type="checkbox" id="xf-elig"> Eligible only</label></div>
<div data-tabmodels hidden style="flex-basis:100%"><label><input type="checkbox" id="xf-pooled"> Pooled only</label> <label><input type="checkbox" id="xf-measured"> AA-measured only</label> <label><input type="checkbox" id="xf-nodis"> Hide disabled providers</label></div>
<div data-tabaa style="flex-basis:100%"><details><summary>Model selection and advanced filters — inequality conditions</summary>
<label>Filter mode <select id="xf-ineq-mode"><option value="off" selected>Off</option><option value="keep">Keep matching models</option><option value="reject">Keep only rejected models</option></select></label>
<label>Combine conditions <select id="xf-ineq-combine"><option value="and" selected>Match all (AND)</option><option value="or">Match any (OR)</option></select></label>
<span class="muted">Pareto-gap metrics compare against the observed frontier of the current scope: score gap (score vs cost), price premium (cost vs score), speed gap (score vs time).</span>
<div id="xf-ineq-conds"></div>
<button type="button" class="xpl-ctl-btn" id="xf-ineq-add">+ Add condition</button>
</details></div>
<div style="flex-basis:100%">
<label>Settings name <input type="text" id="xf-setup-name" placeholder="Name this setup"></label>
<button type="button" class="xpl-ctl-btn" id="xf-setup-save">Save setup</button>
<label>Saved setups <select id="xf-setup-list"></select></label>
<button type="button" class="xpl-ctl-btn" id="xf-setup-load">Load</button>
<button type="button" class="xpl-ctl-btn" id="xf-setup-export">Export settings</button>
<label class="xpl-ctl-btn">Import <input type="file" id="xf-setup-import" accept="application/json" hidden></label>
<button type="button" class="xpl-ctl-btn" id="xf-setup-reset">Reset all settings</button>
<button type="button" class="xpl-ctl-btn" id="xf-dl-json">Download joined data (JSON)</button>
<button type="button" class="xpl-ctl-btn" id="xf-dl-csv">Download visible CSV</button>
</div>
<p class="muted" id="xp-note" style="flex-basis:100%;margin:2px 0 0"></p>
</div>
<div class="card xpl-detail" id="xpl-detail"></div>
<div class="card" id="xpl-tray" hidden><strong>Hidden</strong> <span id="xpl-traychips"></span></div>
<section id="xtab-aa">
${aaTabAge}
<div id="xpl-legend" class="strip"></div>
<div class="xpl-charts" id="xpl-charts">
<div class="chart" data-chart="xpl-c1"><h3>Completion time × Score</h3><span class="xpl-ctl"><label><input type="checkbox" data-sizeby="xpl-c1"> size dots by cost</label><button type="button" data-dl="xpl-c1">Download PNG</button></span><span class="chart-tools" data-tools="xpl-c1"><button type="button" data-zact="expand" data-zfor="xpl-c1">⤢ Expand</button><button type="button" data-zact="in" data-zfor="xpl-c1" aria-label="Zoom in">+</button><button type="button" data-zact="out" data-zfor="xpl-c1" aria-label="Zoom out">−</button><button type="button" data-zact="reset" data-zfor="xpl-c1" aria-label="Reset view">⟲</button></span><svg id="xpl-c1" viewBox="0 0 680 400" role="img" aria-label="Completion time by score scatter chart"></svg><p class="muted" data-note></p></div>
<div class="chart" data-chart="xpl-c2"><h3>Cost × Score</h3><span class="xpl-ctl"><label><input type="checkbox" data-sizeby="xpl-c2"> size dots by time</label><button type="button" data-dl="xpl-c2">Download PNG</button></span><span class="chart-tools" data-tools="xpl-c2"><button type="button" data-zact="expand" data-zfor="xpl-c2">⤢ Expand</button><button type="button" data-zact="in" data-zfor="xpl-c2" aria-label="Zoom in">+</button><button type="button" data-zact="out" data-zfor="xpl-c2" aria-label="Zoom out">−</button><button type="button" data-zact="reset" data-zfor="xpl-c2" aria-label="Reset view">⟲</button></span><svg id="xpl-c2" viewBox="0 0 680 400" role="img" aria-label="Cost by score scatter chart"></svg><p class="muted" data-note></p></div>
<div class="chart" data-chart="xpl-c3"><h3>Cost × Completion time</h3><span class="xpl-ctl"><label><input type="checkbox" data-sizeby="xpl-c3"> size dots by score</label><button type="button" data-dl="xpl-c3">Download PNG</button></span><span class="chart-tools" data-tools="xpl-c3"><button type="button" data-zact="expand" data-zfor="xpl-c3">⤢ Expand</button><button type="button" data-zact="in" data-zfor="xpl-c3" aria-label="Zoom in">+</button><button type="button" data-zact="out" data-zfor="xpl-c3" aria-label="Zoom out">−</button><button type="button" data-zact="reset" data-zfor="xpl-c3" aria-label="Reset view">⟲</button></span><svg id="xpl-c3" viewBox="0 0 680 400" role="img" aria-label="Cost by completion time scatter chart"></svg><p class="muted" data-note></p></div>
<div class="chart" data-chart="xpl-c4"><h3>Tasks completed vs duration</h3><span class="xpl-ctl"><button type="button" data-tmode="per">Seconds per task</button><button type="button" data-tmode="tot">Total time to finish all tasks</button><button type="button" data-dl="xpl-c4">Download PNG</button></span><span class="chart-tools" data-tools="xpl-c4"><button type="button" data-zact="expand" data-zfor="xpl-c4">⤢ Expand</button><button type="button" data-zact="in" data-zfor="xpl-c4" aria-label="Zoom in">+</button><button type="button" data-zact="out" data-zfor="xpl-c4" aria-label="Zoom out">−</button><button type="button" data-zact="reset" data-zfor="xpl-c4" aria-label="Reset view">⟲</button></span><svg id="xpl-c4" viewBox="0 0 680 400" role="img" aria-label="Tasks completed by duration scatter chart"></svg><p class="muted" data-note></p></div>
<div class="chart" data-chart="xpl-c5"><h3>Multiples of the baseline</h3><span class="xpl-ctl"><label>Baseline <select id="xf-baseline"></select></label><button type="button" data-dl="xpl-c5">Download PNG</button></span><span class="chart-tools" data-tools="xpl-c5"><button type="button" data-zact="expand" data-zfor="xpl-c5">⤢ Expand</button><button type="button" data-zact="in" data-zfor="xpl-c5" aria-label="Zoom in">+</button><button type="button" data-zact="out" data-zfor="xpl-c5" aria-label="Zoom out">−</button><button type="button" data-zact="reset" data-zfor="xpl-c5" aria-label="Reset view">⟲</button></span><svg id="xpl-c5" viewBox="0 0 680 400" role="img" aria-label="Cost and duration multiples scatter chart"></svg><p class="muted" data-note></p></div>
<div class="chart" data-chart="xpl-c6"><h3>Scores — every measured configuration</h3><span class="xpl-ctl"><button type="button" data-dl="xpl-c6">Download PNG</button></span><span class="chart-tools" data-tools="xpl-c6"><button type="button" data-zact="expand" data-zfor="xpl-c6">⤢ Expand</button><button type="button" data-zact="in" data-zfor="xpl-c6" aria-label="Zoom in">+</button><button type="button" data-zact="out" data-zfor="xpl-c6" aria-label="Zoom out">−</button><button type="button" data-zact="reset" data-zfor="xpl-c6" aria-label="Reset view">⟲</button></span><svg id="xpl-c6" viewBox="0 0 680 400" role="img" aria-label="Scores bar chart"></svg><p class="muted" data-note></p></div>
</div>
<p class="muted">Time is seconds to finish one task (log scale), not tokens/sec — a model can stream fast and still finish slowly if it burns many tokens, and per-task time here excludes time to first token (shown separately where timed). Thin lines join each model's effort rungs (DeepSeek ladders dotted); the white dashed line is that chart's Pareto frontier; white-ringed dots sit on it, dimmed dots are beaten on its axes. Bracket splits: cost $0.15 / $0.75 / $2.50, time 1m / 5m / 15m. Only measured configurations are plotted — nothing is estimated, so configurations AA never timed appear in the tables, not the time charts. DeepSeek prices split peak / off-peak: peak is 01:00–04:00 and 06:00–10:00 UTC Monday–Friday at double the off-peak token rates, so the same runs appear at two costs. Click a dot or a row for details.</p>
<h2>Multiples table</h2>
<p class="muted">Every visible configuration with all three measures, as multiples of the baseline chosen above, sorted by cost multiple.</p>
<div class="tblwrap"><table class="xpl-table"><caption>Multiples table — every visible configuration as multiples of the baseline</caption><thead><tr><th scope="col">Model</th><th scope="col">Effort</th><th scope="col" class="num">Score</th><th scope="col" class="num">Score ×</th><th scope="col" class="num xpl-costcol">Cost/task</th><th scope="col" class="num xpl-costcol">Cost ×</th><th scope="col" class="num">Time/task</th><th scope="col" class="num">Time ×</th><th scope="col">Benchmark</th></tr></thead><tbody id="xpl-mtrows"></tbody></table></div>
<h2>Configurations</h2>
<p class="muted" id="xpl-count"></p>
<div class="tblwrap"><table class="xpl-table"><caption>Configurations — intelligence, cost, and speed per model</caption><thead><tr><th scope="col">Model</th><th scope="col">Effort</th><th scope="col" class="num" id="xpl-scorehead">Intelligence</th><th scope="col" class="num xpl-costcol">$/task</th><th scope="col" class="num">Time/task</th><th scope="col" class="num">TTFT</th><th scope="col" class="num">Speed (t/s)</th><th scope="col" class="num xpl-costcol">Score / $</th><th scope="col" class="num">Score / s</th></tr></thead><tbody id="xpl-rows"></tbody></table></div>
<h2>Pareto frontiers</h2>
<p class="muted">Each list keeps variants that are not dominated on its named dimensions: another variant cannot be at least as good on every dimension and strictly better on one. The first two lists are the same score–speed frontier in different priority orders. Percentile ranks use the current filtered models; 100% is best. Missing measures are excluded only from a frontier that needs them.</p>
<label>Minimum score and speed percentile in the price shortlist <select id="xf-pfcut"><option value="50" selected>At least median · 50th percentile</option><option value="60">At least 60th percentile</option><option value="75">Top quarter · 75th percentile</option><option value="0">No cutoff · full Pareto frontier</option></select></label>
<p class="muted" id="xpl-pf-count"></p>
<h3>Score first, with competitive time</h3>
<div class="tblwrap"><table class="xpl-table"><caption>Pareto frontier — score first, with competitive time</caption><thead><tr><th scope="col">Model / effort</th><th scope="col" class="num">Score · value / percentile</th><th scope="col" class="num">Time / task · value / percentile</th><th scope="col" class="num xpl-costcol">Cost / task · value / percentile</th></tr></thead><tbody id="xpl-pf-score"></tbody></table></div>
<h3>Time first, with competitive score</h3>
<div class="tblwrap"><table class="xpl-table"><caption>Pareto frontier — time first, with competitive score</caption><thead><tr><th scope="col">Model / effort</th><th scope="col" class="num">Score · value / percentile</th><th scope="col" class="num">Time / task · value / percentile</th><th scope="col" class="num xpl-costcol">Cost / task · value / percentile</th></tr></thead><tbody id="xpl-pf-speed"></tbody></table></div>
<h3>Price first, strong in score and time</h3>
<div class="tblwrap"><table class="xpl-table"><caption>Pareto frontier — price first, strong in score and time</caption><thead><tr><th scope="col">Model / effort</th><th scope="col" class="num">Score · value / percentile</th><th scope="col" class="num">Time / task · value / percentile</th><th scope="col" class="num xpl-costcol">Cost / task · value / percentile</th></tr></thead><tbody id="xpl-pf-price"></tbody></table></div>
<h3>Price–speed trade-offs</h3>
<div class="tblwrap"><table class="xpl-table"><caption>Pareto frontier — price and speed trade-offs</caption><thead><tr><th scope="col">Model / effort</th><th scope="col" class="num">Time / task</th><th scope="col" class="num xpl-costcol">Cost / task</th><th scope="col" class="num">Score · context</th></tr></thead><tbody id="xpl-pf-ps"></tbody></table></div>
<h2>Recommendations</h2>
<p class="muted">Build a useful shortlist: first require a near-top score in each band, then trade price against speed and choose distinct labs. Score bands are non-overlapping and calculated from the filtered population before tradeoff limits; absolute limits apply first, then relative price/speed limits against the cheapest and fastest near-top models passing them. Unknown selected values cannot be recommended.</p>
<div class="card xpl-filters">
<span role="group" aria-label="Recommendation presets"><button type="button" class="xpl-ctl-btn" data-recpreset="fast">Fast near-top</button><button type="button" class="xpl-ctl-btn" data-recpreset="balanced">Balanced</button><button type="button" class="xpl-ctl-btn" data-recpreset="diversity">More lab diversity</button><button type="button" class="xpl-ctl-btn" data-recpreset="budget">Tight budget</button><button type="button" class="xpl-ctl-btn" data-recpreset="reset">Reset tradeoffs</button></span>
<label>Score bands <select id="xf-bands"><option value="2">2 tiers</option><option value="3" selected>3 tiers</option><option value="4">4 tiers</option><option value="5">5 tiers</option><option value="6">6 tiers</option><option value="8">8 tiers</option></select></label>
<label>Allow score below band best <input type="number" data-rec="allow" value="5" min="0" step="1" style="width:4em"> points</label>
<label>Favor price ← → favor speed <input type="range" data-rec="speedPriority" min="0" max="100" step="5" value="85"> <span id="rec-speedpriority-val">85% speed</span></label>
<label>Extra price above cheapest near-top <input type="number" data-rec="priceAllow" value="500" min="0" step="50" style="width:5em"> %</label>
<label>Slowest allowed vs fastest near-top <input type="number" data-rec="slowAllow" value="2" min="1" step="0.1" style="width:4em"> ×</label>
<label>Allow alternatives within <input type="number" data-rec="altWindow" value="2" min="1" step="0.1" style="width:4em"> × best tradeoff</label>
<label>Lab diversity <select data-rec="diversity"><option value="lab" selected>One choice per lab</option><option value="labsfirst">Different labs first</option><option value="none">No lab grouping</option></select></label>
<label>Remove dominated models <select data-rec="dominance"><option value="lab" selected>Within each lab</option><option value="all">Across all labs · strict Pareto</option><option value="none">Do not remove</option></select></label>
<label>At most per band <select data-rec="perBand"><option value="1">1</option><option value="2">2</option><option value="3" selected>3</option><option value="4">4</option><option value="5">5</option></select></label>
<label>Maximum USD / task <input type="number" data-rec="maxCost" placeholder="none" min="0" step="0.1" style="width:5em"></label>
<label>Maximum seconds / task <input type="number" data-rec="maxSec" placeholder="none" min="0" step="10" style="width:5em"></label>
<button type="button" class="xpl-ctl-btn" id="xpl-rec-export">Export shortlist CSV</button>
</div>
<p class="muted" id="xpl-rec-summary"></p>
<div id="xpl-recs"></div>
<h2>Band comparisons</h2>
<p class="muted">Best choices within each score and cost band; shortlists follow the current model and effort filters. Each band row shows the observed range of its other two measures with the endpoint configurations named.</p>
<label>Near-leader window <select id="xf-bandscale"><option value="0.5">0.5× · Very tight</option><option value="0.75">0.75× · Tight</option><option value="1" selected>1× · Standard (score ±2 · time within 10% · price within 25%)</option><option value="1.5">1.5× · Generous</option><option value="2">2× · Wide</option><option value="3">3× · Broad</option></select></label>
<div id="xpl-bandgroups"></div>
<h2>Timing — Test_Runs</h2>
<p class="muted">Per-configuration timing from the import's Test_Runs data. "Not measured" means the import carries no value for that cell — nothing is estimated.</p>
${aaTimingsTable(s.testRuns)}
<h2>Measured configurations</h2>
<p class="muted">The AA Intelligence Index rows of the latest import batch, effort labels verbatim from the import.</p>
${aaConfigsTable(s.configs)}
${s.cai ? aaCaiTable(s.cai) : ""}
</section>
<section id="xtab-models" hidden>
${catalogTabAge}
<h2>The fleet's model universe</h2>
<p class="muted">Every model in the fleet config's provider catalogs: the plan each door belongs to and its effective value multiplier where the registry records one, the pools that seat it, and its AA measurement when the import has one. Click a row for the plan note.</p>
${modelCards}
<h3>Task projection — estimated, not measured</h3>
<p class="muted">The task-assumptions projection from the ChatGPT coding explorer: set the shape of a task and every model with a recorded token rate gets an estimated cost and time, marked ~. Prices come only from the ainventory price event log joined onto this catalog (the band your per-turn input falls in); a model with no recorded rate — or missing a rate the assumptions need — shows —, never a guess. Time borrows the family's measured TTFT and throughput. Cache-storage hours are not modelled: the price log carries no storage rate.</p>
<div class="card xpl-filters">
<span role="group" aria-label="Task presets"><button type="button" class="xpl-ctl-btn" data-pjpreset="small">Small patch</button><button type="button" class="xpl-ctl-btn" data-pjpreset="feature">Feature</button><button type="button" class="xpl-ctl-btn" data-pjpreset="repo">Repo task</button></span>
<label>Turns <input type="number" data-pj="turns" value="3" min="1" style="width:4em"></label>
<label>Input / turn <input type="number" data-pj="inputTokens" value="20000" step="1000" style="width:6em"></label>
<label>Answer / turn <input type="number" data-pj="answerTokens" value="2000" step="500" style="width:5em"></label>
<label>Thinking / turn <input type="number" data-pj="thinkingTokens" value="4000" step="500" style="width:5em"></label>
<label>Cache read share <input type="range" data-pj="cacheReadPct" min="0" max="100" step="5" value="50"> <span id="pj-cacheread-val">50%</span></label>
<label>Writes / uncached <input type="number" data-pj="writesPct" value="0" min="0" max="100" style="width:4em"> %</label>
<label>Tool overhead / turn <input type="number" data-pj="overheadSec" value="10" min="0" style="width:4em"> s</label>
<label>Rate window <select data-pj="window"><option value="peak" selected>Peak (conservative)</option><option value="offpeak">Off-peak</option></select></label>
<span class="muted">The window applies to scheduled rates only (DeepSeek halves every token rate off-peak).</span>
</div>
<p class="muted" id="xpl-mcount"></p>
<div class="tblwrap"><table class="xpl-table"><caption>Model catalog — providers, plans, and projected costs</caption><thead><tr><th scope="col">Model</th><th scope="col">Provider / plan</th><th scope="col" class="num">Context</th><th scope="col">Variants</th><th scope="col">Pool seats</th><th scope="col" class="num">AA score</th><th scope="col" class="num xpl-costcol">AA $/task</th><th scope="col" class="num xpl-costcol">Eff. $/task</th><th scope="col" class="num">AA time/task</th><th scope="col" class="num xpl-costcol">Proj. $/task ~</th><th scope="col" class="num">Proj. time/task ~</th><th scope="col" class="num xpl-costcol">AA score / proj. $</th></tr></thead><tbody id="xpl-mrows"></tbody></table></div>
</section>
${coverageHtml}
<script type="application/json" id="xpl-data">${explorerJson}</script>`;
  return layout("Cost Explorer", body, provenance, "/aa", AA_EXPLORER_SCRIPT, aaFreshnessChips(detail, section));
}

// --- Sessions / identity registry page (#906 §3) ------------------------------

const REGISTRY_SOURCE_ORDER = ["opencode-laptop", "opencode-sandbox", "jules", "claude-code", "cron"];

export function renderSessions(
  records: SessionRecord[],
  provenance: Provenance,
  detail?: ProvenanceDetail,
  act?: { actionsByObject?: Map<string, DashAction[]>; identities?: AuthorIdentity[] },
  view?: RegistryView,
): string {
  const bySource = new Map<string, SessionRecord[]>();
  for (const r of records) bySource.set(r.source, [...(bySource.get(r.source) ?? []), r]);
  const totalTokens = records.reduce((sum, r) => sum + (r.tokensTotal ?? 0), 0);
  const groups = [...bySource.entries()]
    .sort((a, b) => REGISTRY_SOURCE_ORDER.indexOf(a[0]) - REGISTRY_SOURCE_ORDER.indexOf(b[0]))
    .map(
      ([source, list]) =>
        `<h2>${esc(source)} (${list.length})</h2>${list
          .map((r) =>
            sessionRecordLine(r, {
              actions: act?.actionsByObject?.get(sessionObjectRef(r.id)),
              identities: act?.identities,
            }),
          )
          .join("")}`,
    )
    .join("");
  // #1209: when the server hands us a RegistryView, the page is one
  // windowed/filtered/paginated slice of the store — say so, with the
  // hidden counts, a search/filter form, and pagination links. Without
  // a view (older callers/tests) the page renders what it was given.
  let viewHtml = "";
  if (view) {
    const q = view.query;
    const windowLabel = q.windowDays === null ? "all history" : `last ${q.windowDays} days`;
    const filterBits = [
      q.q ? `search "${esc(q.q)}"` : "",
      q.agent ? `agent "${esc(q.agent)}"` : "",
      q.task ? `task "${esc(q.task)}"` : "",
    ].filter(Boolean);
    const hiddenBits = [
      view.hiddenByWindow > 0 ? `${view.hiddenByWindow} hidden by the ${windowLabel} window` : "",
      view.hiddenByPagination > 0 ? `${view.hiddenByPagination} more on other pages` : "",
    ].filter(Boolean);
    const toggle =
      q.windowDays === null
        ? `<a href="/sessions${registryQueryString(q, { windowDays: 7, offset: 0 })}">last 7 days</a>`
        : `<a href="/sessions${registryQueryString(q, { windowDays: null, offset: 0 })}">all history</a>`;
    const prev =
      q.offset > 0
        ? `<a href="/sessions${registryQueryString(q, { offset: Math.max(0, q.offset - q.limit) })}">← prev</a>`
        : "";
    const next =
      view.remainingAfterPage > 0
        ? `<a href="/sessions${registryQueryString(q, { offset: q.offset + q.limit })}">next →</a>`
        : "";
    viewHtml = `<div class="card"><strong>${view.matched} of ${view.totalAll} sessions</strong> · window: ${esc(windowLabel)}${filterBits.length ? ` · ${filterBits.join(" · ")}` : ""}${hiddenBits.length ? `<br><span class="muted">${hiddenBits.join(" · ")}</span>` : ""}${view.pinnedIncluded ? `<br><span class="muted">A deep-linked session outside this view is pinned at the top of the list.</span>` : ""}<br>${toggle}${prev ? ` · ${prev}` : ""}${next ? ` · ${next}` : ""}
<form method="get" action="/sessions"><label>Search <input type="search" name="q" value="${esc(q.q ?? "")}"></label> <label>Agent <input type="text" name="agent" value="${esc(q.agent ?? "")}"></label> <label>Task ref <input type="text" name="task" value="${esc(q.task ?? "")}"></label> <label>Window <select name="window"><option value="7"${q.windowDays === 7 ? " selected" : ""}>last 7 days</option><option value="1"${q.windowDays === 1 ? " selected" : ""}>last 24 hours</option><option value="30"${q.windowDays === 30 ? " selected" : ""}>last 30 days</option><option value="90"${q.windowDays === 90 ? " selected" : ""}>last 90 days</option><option value="all"${q.windowDays === null ? " selected" : ""}>all history</option></select></label> <button type="submit">Filter</button></form></div>`;
  }
  const body = `<h1>Sessions — identity registry</h1>
${viewHtml}<p class="muted">Every agent/session the fleet can see, with its stable id, the agent identity it ran as, its parent session, and its token total where the source reports one. Task links are loose (from the session's PR, title, or launch record) and improve over time. ${records.length} records shown · ${totalTokens.toLocaleString()} tokens recorded on this page. Machine-first: <a href="/api/registry.json">/api/registry.json</a>.</p>
<p class="muted">Coverage honesty: OpenCode sessions register from their servers; Muse sessions only when a launch record was appended at spawn (<code>scripts/record-claude-launch.ts</code>) — a Muse session that never wrote one is invisible here, and the coverage note on Needs you says which sources are wired.</p>
${groups || '<p class="muted">No sessions recorded yet.</p>'}`;
  return layout(
    "Sessions",
    body,
    provenance,
    "/sessions",
    act?.identities ? ACTION_SCRIPT : "",
    detail ? freshnessChips(detail, ["registry"]) : "",
  );
}

// --- All pages (the manifest as a page) ---------------------------------------

// --- Source-conflict inbox (#1282, slice 1) -----------------------------------

/**
 * /conflicts — the read-only source-conflict inbox. Every disagreement the
 * facets already report, in one queue, grouped by conflict class, each entry
 * naming the item, both sources, both values and how long the pair has been
 * on record. It resolves nothing: slice 1 projects only, so this page is the
 * same list /api/conflicts.json serves, rendered.
 */
export function renderConflicts(inbox: ConflictInbox, provenance: Provenance, detail?: ProvenanceDetail): string {
  const chips = detail ? freshnessChips(detail, ["items", "usage"]) : "";
  const ageCell = (entry: ConflictEntry): string => {
    if (entry.ageDays === null) return `<span class="muted">undated</span>`;
    const days = `<strong>${entry.ageDays}</strong> day${entry.ageDays === 1 ? "" : "s"}`;
    const floor = entry.ageBasis === "project-item-updated-at" ? " (floor)" : "";
    return `at least ${days}${floor}<br><span class="muted">${esc(fmtDate(entry.observedAt ?? ""))}${entry.ageBasis === "undated" ? "" : ` · ${esc(entry.ageBasis)}`}</span>`;
  };
  const itemCell = (entry: ConflictEntry): string => {
    const label = esc(entry.item.title);
    if (!entry.item.url) return `<strong>${label}</strong>`;
    return `<a href="${esc(entry.item.url)}"><strong>${label}</strong></a>${entry.item.ref ? `<br><span class="muted"><code>${esc(entry.item.ref)}</code></span>` : ""}`;
  };
  const groups = inbox.byClass
    .map((cls) => {
      const entries = inbox.entries.filter((e) => e.classId === cls.classId);
      if (!entries.length) return "";
      const rows = entries
        .map(
          (e) =>
            `<tr><td>${itemCell(e)}</td><td>${esc(e.sources.a)}<br><span class="muted">vs</span><br>${esc(e.sources.b)}</td><td><span class="badge stale">${esc(e.values.a)}</span><br><span class="muted">vs</span><br><span class="badge stale">${esc(e.values.b)}</span></td><td>${ageCell(e)}</td><td>${e.href ? `<a href="${esc(e.href)}">records →</a>` : "—"}</td></tr>`,
        )
        .join("");
      return `<h2 id="class-${esc(cls.classId)}">${esc(cls.label)} (${entries.length})</h2>
<div class="tblwrap"><table class="table table-zebra"><caption>${esc(cls.label)} — item, sources, both values, age</caption><tr><th scope="col">Item</th><th scope="col">Sources</th><th scope="col">Both values</th><th scope="col">Disagreeing for</th><th scope="col">Records</th></tr>${rows}</table></div>`;
    })
    .join("");
  const oldest =
    inbox.oldestAgeDays === null ? "—" : `at least ${inbox.oldestAgeDays} day${inbox.oldestAgeDays === 1 ? "" : "s"}`;
  const body = `<h1>Source conflicts</h1>
<p class="muted">Every live disagreement the facets already report, in one queue: <strong>${inbox.total}</strong> entr${inbox.total === 1 ? "y" : "ies"} across ${inbox.byClass.filter((c) => c.count > 0).length} class${inbox.byClass.filter((c) => c.count > 0).length === 1 ? "" : "es"} · oldest ${oldest} · assembled ${esc(fmtDate(inbox.generatedAt))}. Machine-first: <a href="/api/conflicts.json">/api/conflicts.json</a>. This page projects; it does not resolve — trust rulings are slice 2 of #1282, so nothing here changes what a render shows.</p>
<div class="board">
<div class="card"><strong>Conflicts</strong><br>${inbox.total} live</div>
${inbox.byClass.map((c) => `<div class="card"><strong>${esc(c.label)}</strong><br>${c.count} live</div>`).join("")}
<div class="card"><strong>Oldest disagreement</strong><br>${oldest}</div>
</div>
<p class="muted">Age is a <strong>floor</strong>, not an exact birthday: it is measured from the newest timestamp the source carries for the pair (a Project item's <code>updatedAt</code>, a service's <code>measuredAt</code>), so the values have disagreed for <em>at least</em> that long — an item edited yesterday for another reason can carry a much older clash.</p>
${inbox.total ? groups : `<div class="banner">No live source disagreements in this projection — every facet that reports a disagreement agrees with its counterpart right now. An empty inbox here is a real empty, not an unread one: the page and /api/conflicts.json are built from the same projection.</div>`}`;
  return layout("Source conflicts", body, provenance, "/conflicts", "", chips);
}

export function renderPages(provenance: Provenance, history: PageHealth[] = []): string {
  const byRoute = new Map(history.map((p) => [p.route, p]));
  const rows = PAGES.map((p) => {
    const h = byRoute.get(p.route);
    const last =
      !h || h.lastRenderedAt === null
        ? '<span class="muted">not rendered since this process started</span>'
        : `${esc(ageMsText(h.ageMs ?? 0))} · ${h.ok ? "ok" : '<span class="badge stale">last render failed</span>'}`;
    const tier = p.nav === "primary" ? "Primary nav" : "Dropdown-only";
    return `<tr><td><a href="${p.route}">${esc(p.title)}</a></td><td><code>${esc(p.route)}</code></td><td>${p.api ? `<a href="${p.api}"><code>${esc(p.api)}</code></a>` : '<span class="muted">—</span>'}</td><td>${tier}</td><td>${last}</td><td>${esc(p.blurb)}</td></tr>`;
  }).join("");
  return layout(
    "All pages",
    `<h1>All pages</h1><p class="muted">Every registered page, from the manifest in <code>src/pages.ts</code> — the nav bar renders the primary tier, the page dropdown lists them all (#1497). Last-rendered + status come live from the render registry (#1147), the same one <a href="/health">Health</a> reports — a page this process has not rendered yet says so, never a fake time. Adding a page is one manifest entry + a render function + a route (recipe in the README).</p><div class="tblwrap"><table class="table table-zebra"><caption>All registered pages</caption><tr><th scope="col">Page</th><th scope="col">Route</th><th scope="col">JSON twin</th><th scope="col">Nav tier</th><th scope="col">Last rendered</th><th scope="col">What it is</th></tr>${rows}</table></div>`,
    provenance,
    "/pages",
  );
}

// --- Health & topology (#1147 page, #1137 table) ---------------------------

/**
 * The /health page: one view assembled by src/health.ts from live
 * probes (cached 60 s), the feed, the section diagnosis, and this
 * process's render registry — the same view /api/health serves, so
 * the page and the API cannot disagree. States are named per row; a
 * failed probe, a stale feed, and an unrendered page all say so.
 */
export function renderHealth(view: HealthView, provenance: Provenance): string {
  const nowMs = Date.parse(view.generatedAt);
  const probedAge = (iso?: string): string =>
    iso ? ` · probed ${ageMsText(Math.max(0, nowMs - Date.parse(iso)))}` : "";
  const serviceRows = view.services
    .map((s) => {
      const port = s.def.port !== null ? String(s.def.port) : "—";
      const upWhen = s.def.upWhen ? `<br><span class="muted">up = ${esc(s.def.upWhen)}</span>` : "";
      const notes = s.def.notes ? `<br><span class="muted">${esc(s.def.notes)}</span>` : "";
      return `<tr><td><strong>${esc(s.def.name)}</strong>${upWhen}${notes}</td><td><code>${port}</code></td><td>${esc(s.def.bind)}</td><td><code>${esc(s.def.runsAs)}</code></td><td><span class="dot ${s.state}"></span><strong>${esc(s.state)}</strong> — ${esc(s.detail)}${probedAge(s.probedAt)}</td><td>${esc(s.def.consumers)}</td><td>${esc(s.def.breaksIfDown)}</td></tr>`;
    })
    .join("");
  const sectionRows = view.feed.sections.length
    ? view.feed.sections
        .map((s) => {
          let status: string;
          if (s.status === "unknown") status = s.present ? "present" : '<span class="badge stale">absent</span>';
          else if (s.status === "error") status = '<span class="badge stale">error</span>';
          else if (s.status === "skipped") status = '<span class="muted">skipped</span>';
          else status = s.present ? "present · ok" : "empty · ok";
          return `<tr><td><code>${esc(s.key)}</code></td><td>${status}</td><td>${s.ageMs !== undefined ? esc(ageMsText(s.ageMs)) : "—"}</td><td>${s.note ? esc(s.note) : ""}</td></tr>`;
        })
        .join("")
    : `<tr><td colspan="4" class="muted">No readable fleet feed — sections cannot be reported.</td></tr>`;
  const orch = view.jobs.orchestration;
  const orchLine = orch.generatedAt
    ? `last run <code>${esc(orch.runId ?? "unknown")}</code> · ${esc(ageMsText(orch.ageMs ?? 0))} · state <strong>${esc(orch.state)}</strong>${orch.failedChecks ? ` · failed checks: ${esc(orch.failedChecks.join(", "))}` : ""} · next expected ~${esc(orch.nextExpectedAt ?? "")} (hourly cadence — an expectation, not a promise)`
    : `no run on record · state <strong>${esc(orch.state)}</strong>${orch.failedChecks ? ` · failed checks: ${esc(orch.failedChecks.join(", "))}` : ""}`;
  const coll = view.jobs.collector;
  const drift = view.feed.checkout;
  const driftLine =
    drift.state === "behind"
      ? ` · <span class="badge stale">collector checkout behind origin/main by ${drift.behindBy ?? "?"} commit(s)</span> — the collector is running old code`
      : drift.state === "in-sync"
        ? ` · collector checkout in sync with origin/main`
        : ` · collector checkout drift unknown: ${esc(drift.reason ?? "no reason given")}`;
  const collLine = coll.generatedAt
    ? `feed generated ${esc(ageMsText(coll.ageMs ?? 0))} · file written ${coll.fileMtime ? esc(coll.fileMtime) : "unknown"}${driftLine}`
    : "no readable fleet feed";
  const pageRows = view.pages
    .map((p) => {
      const last =
        p.lastRenderedAt === null
          ? `<span class="muted">not rendered since this process started (boot ${esc(view.bootAt)})</span>`
          : `${esc(ageMsText(p.ageMs ?? 0))} · ${p.ok ? "ok" : '<span class="badge stale">last render failed</span>'}`;
      return `<tr><td><a href="${esc(p.route)}">${esc(p.title)}</a></td><td><code>${esc(p.route)}</code></td><td>${last}</td></tr>`;
    })
    .join("");
  // #1203: the ready-for-agent facet — the harness's real dispatch-layer
  // state, not just whether its UI answers 200. "down" means the GraphQL
  // read failed (detail names the error) or a backend is not READY; the
  // repositories table still shows what the harness reports when it can
  // be read at all.
  const rfa = view.rfa;
  const rfaState =
    rfa.state === "up"
      ? '<span class="dot up"></span><strong>up</strong>'
      : rfa.state === "fixtures"
        ? '<span class="dot fixtures"></span><strong>fixtures</strong>'
        : '<span class="dot down"></span><strong>down</strong>';
  const rfaRepoRows = rfa.repositories.length
    ? rfa.repositories
        .map(
          (r) =>
            `<tr><td><code>${esc(r.projectPath)}</code></td><td>${r.paused ? "paused" : "running"}</td><td><code>${esc(r.mergePolicy ?? "—")}</code></td><td>${r.workItems}</td></tr>`,
        )
        .join("")
    : `<tr><td colspan="4" class="muted">No harness repositories reported — the harness could not be read, or none are registered.</td></tr>`;
  const rfaBackendLine = rfa.backends.length
    ? `Backends: ${rfa.backends.map((b) => `<code>${esc(b.id)}</code> ${esc(b.kind)}${b.reason ? ` — ${esc(b.reason)}` : ""}`).join(" · ")}`
    : "Backends: none reported.";
  const rfaSection = `<h2 id="ready-for-agent">ready-for-agent harness</h2>
<div class="card">${rfaState} — ${esc(rfa.detail)}<br>${rfaBackendLine}</div>
<div class="tblwrap"><table class="table table-zebra"><caption>ready-for-agent repositories — settings and dispatch work items</caption><tr><th scope="col">Repository</th><th scope="col">Paused</th><th scope="col">Merge policy</th><th scope="col">Work items</th></tr>${rfaRepoRows}</table></div>`;
  const cov = view.itemsCoverage;
  const coverageLine =
    cov && cov.total > 0
      ? `<strong>${cov.resolvedLive} of ${cov.total}</strong> board items resolved from live data (project pool / projector) · <strong>${cov.fixture}</strong> resolved from fixtures (rendered hollow + "unverified") · coverage ratio <strong>${cov.ratio !== null ? cov.ratio.toFixed(2) : "—"}</strong> — measured on the health prime's board resolution pass, the same pass that sets the items facet provenance (#1141).`
      : `not measured yet — coverage is computed on the health prime's board resolution pass (#1267); a pass that resolved no tagged items reports nothing rather than inventing a ratio.`;
  const facets = view.facets
    ? freshnessChips(view.facets, [
        "board",
        "roadmap",
        "items",
        "comments",
        "quota",
        "usage",
        "attention",
        "orchestration",
        "dns",
        "registry",
        "aa",
      ])
    : `<p class="muted">This source does not report structured source facets.</p>`;
  const degraded = view.degraded
    ? `<div class="flag">Something below is degraded — each row names its own state; nothing on this page is smoothed over.</div>`
    : "";
  const modeNote =
    view.mode === "fixtures"
      ? `<div class="banner">Fixtures mode — no live probes are made in this deployment; every service reports “fixtures”.</div>`
      : "";
  const body = `<h1>Health</h1>
<p class="muted">Every stack service probed live (results cached 60 s), every feed section aged, the last job runs, and this dashboard's own render history — assembled from the same view <a href="/api/health">/api/health</a> serves, at ${esc(view.generatedAt)}.</p>
${degraded}${modeNote}
<h2 id="topology">Services &amp; topology</h2>
<p class="muted">What runs where: the component, its port and bind as actually deployed, what consumes it, and what breaks without it. State is probed, never assumed.</p>
<div class="tblwrap"><table class="table table-zebra"><caption>Services, ports, and topology</caption><tr><th scope="col">Service</th><th scope="col">Port</th><th scope="col">Bind</th><th scope="col">Runs as</th><th scope="col">State</th><th scope="col">Consumed by</th><th scope="col">If it is down</th></tr>${serviceRows}</table></div>
${rfaSection}
<h2>Feed sections</h2>
<div class="tblwrap"><table class="table table-zebra"><caption>Feed sections — presence and age</caption><tr><th scope="col">Section</th><th scope="col">Present</th><th scope="col">Age</th><th scope="col">Note</th></tr>${sectionRows}</table></div>
<h2>Jobs</h2>
<div class="card"><strong>Orchestration pass</strong><br>${orchLine}</div>
<div class="card"><strong>Fleet-feed collector</strong><br>${collLine}</div>
<h2>Item resolution coverage</h2>
<div class="card">${coverageLine}</div>
<h2>Source conflicts</h2>
<div class="card">${
    view.conflicts
      ? `<strong><a href="/conflicts">${view.conflicts.total} live source conflict${view.conflicts.total === 1 ? "" : "s"}</a></strong>${view.conflicts.byClass
          .filter((c) => c.count > 0)
          .map((c) => ` · ${esc(c.label)}: ${c.count}`)
          .join(
            "",
          )}${view.conflicts.oldestAgeDays === null ? "" : ` · oldest at least <strong>${view.conflicts.oldestAgeDays}</strong> day${view.conflicts.oldestAgeDays === 1 ? "" : "s"}`} (#1282 slice 1 — a growing count means the sources keep disagreeing and nobody has ruled on the class)`
      : `<span class="muted">not assembled for this health view</span>`
  }</div>
<h2>Dashboard comments</h2>
<div class="card"><strong>${view.comments.pendingMessages} message${view.comments.pendingMessages === 1 ? "" : "s"} awaiting triage</strong> · <strong>${view.comments.pendingGithub} GitHub comment${view.comments.pendingGithub === 1 ? "" : "s"} awaiting posting</strong> · ${view.comments.total} comment${view.comments.total === 1 ? "" : "s"} in the store (#1145 — a pending count that does not drain means work arrives faster than it is cleared, or the triage/poster loop has stopped; check the loop's last run before acting)</div>
<h2>Source facets</h2>
${facets}
<h2>Pages</h2>
<p class="muted">Render history of this process — a page nobody has opened since boot is labelled unrendered, not green.</p>
<div class="tblwrap"><table class="table table-zebra"><caption>Pages — last rendered</caption><tr><th scope="col">Page</th><th scope="col">Route</th><th scope="col">Last rendered</th></tr>${pageRows}</table></div>`;
  return layout("Health", body, provenance, "/health");
}

// --- Discuss page (one object's dashboard comments) -----------------------------

export function renderDiscuss(
  objectRef: string,
  title: string,
  comments: DashComment[],
  opts: CommentFormOpts,
  provenance: Provenance,
): string {
  const back =
    objectRef.startsWith("gh:") || objectRef.startsWith("kaneo:") || objectRef.startsWith("grist:")
      ? `<p class="muted"><a href="/item/${encodeURIComponent(objectRef)}">← the item page</a></p>`
      : objectRef.startsWith("session:")
        ? `<p class="muted"><a href="/sessions">← the session registry</a> · <a href="/attention">Needs you</a></p>`
        : `<p class="muted"><a href="/attention">← Needs you</a></p>`;
  const body = `<h1>${esc(title)}</h1><p class="muted">object: <code>${esc(objectRef)}</code></p>${back}
<p class="muted">Two kinds, visibly distinct: a <strong>note</strong> is read when this is next picked up and triggers nothing; a <strong>message to Muse</strong> is triaged by the orchestrator on its next pass and may create work.${
    opts.githubRefs?.length
      ? ` This item is GitHub-backed, so a comment can also be posted as a <strong>GitHub comment</strong> on an explicit target.`
      : ""
  }</p>
${commentThreadHtml(objectRef, comments, opts)}`;
  return layout(title, body, provenance, "", COMMENT_SCRIPT);
}

// --- Comments queue page (#1210) ------------------------------------------

/** The queue view: every comment still waiting on a loop — messages
 *  awaiting triage and GitHub comments pending/failed to post — with
 *  kind/state counts. Terminal comments (triaged/posted) and notes
 *  are not queued and never render here. */
export function renderCommentsQueue(comments: DashComment[], provenance: Provenance): string {
  const queued = queuedComments(comments);
  const k = commentKindCounts(comments);
  const summary = `${queued.length} queued — ${k.messagesNew} messages awaiting triage · ${k.githubPending} GitHub comments to post · ${k.githubFailed} failed to post <span class="muted">(of ${k.total} comments total: ${k.notes} notes, ${k.messages} messages, ${k.github} GitHub)</span>`;
  const list = queued.length
    ? queued
        .map(
          (c) =>
            `${dashCommentHtml(c, true)}\n<p class="muted"><a href="/discuss/${encodeURIComponent(c.objectRef)}">open the thread</a></p>`,
        )
        .join("\n")
    : `<p class="muted">Nothing is queued — every message is triaged and every GitHub comment is posted.</p>`;
  const body = `<h1>Comments queue</h1>
<p class="muted">Every dashboard comment still waiting on a loop (#1210), oldest first. A <strong>note</strong> is never queued: it is read when its item is next picked up. Machine-first: <a href="/api/comments/queue.json">/api/comments/queue.json</a>.</p>
<p>${summary}</p>
${list}`;
  return layout("Comments queue", body, provenance, "/comments");
}

// --- Projects page (#910 stage 5: the live GitHub Project pool) ------------
// New function only (parallel agents own the other render functions).
// Renders the feed's Project pool; where the Project Status and the
// derived state (see src/projects.ts derivedProjectState) disagree,
// BOTH are shown — neither side is silently preferred.

/** #1220: the coverage panel — Project #4 pool vs backlog reality per
 * repo (open issues and gate labels from the feed, pool size and unset
 * Status/Area from the pool), with the week-over-week delta against the
 * stored baseline in parentheses when a baseline exists. */
function coverageSectionHtml(coverage: CoverageSnapshotFile | null | undefined): string {
  if (!coverage) return "";
  const { current, baseline } = coverage;
  const delta = diffCoverage(current, baseline);
  const cell = (
    repo: string,
    v: number | null,
    key: "openIssues" | "gateNow" | "gateNext" | "poolItems" | "poolUnsetStatus" | "poolUnsetArea",
  ): string => {
    if (v === null) return "—";
    const d = delta.get(repo)?.[key];
    return d === undefined || d === null || d === 0
      ? `${v}`
      : `${v} <span class="muted">(${d > 0 ? "+" : ""}${d})</span>`;
  };
  const rows = current.repos
    .map(
      (r) =>
        `<tr><td>${esc(r.repo)}</td><td>${cell(r.repo, r.openIssues, "openIssues")}</td><td>${cell(r.repo, r.gateNow, "gateNow")}</td><td>${cell(r.repo, r.gateNext, "gateNext")}</td><td>${cell(r.repo, r.poolItems, "poolItems")}</td><td>${cell(r.repo, r.poolUnsetStatus, "poolUnsetStatus")}</td><td>${cell(r.repo, r.poolUnsetArea, "poolUnsetArea")}</td></tr>`,
    )
    .join("");
  const note = baseline
    ? `Week-over-week delta vs the ${esc(fmtDate(baseline.generatedAt))} snapshot in parentheses. Snapshot stored ${esc(fmtDate(current.generatedAt))}; it rotates into the baseline after a week.`
    : `First snapshot stored ${esc(fmtDate(current.generatedAt))} — the week-over-week delta appears after the next weekly rotation.`;
  return `<h2>Coverage — pool vs backlog (#1220)</h2>
 <div class="tblwrap"><table><caption>Coverage — pool vs backlog by repository</caption><thead><tr><th scope="col">Repo</th><th scope="col">Open issues</th><th scope="col">now gate</th><th scope="col">next gate</th><th scope="col">Pool items</th><th scope="col">Unset Status</th><th scope="col">Unset Area</th></tr></thead><tbody>${rows}</tbody></table></div>
<p class="muted">${note} Open issues and gate labels come from the collector's Search API census in the fleet feed; pool counts from Project #4 (— = that source was unavailable for the repo, never a real zero). The same report is in <a href="/api/projects.json">/api/projects.json</a> as \`coverage\`.</p>`;
}

/** The Projects pool body (cards, chips, coverage) without the page
 * shell — rendered standalone by renderProjects and, since the #1497
 * merge, as the pool section of /program. */
/** The per-item brief panel (#1279): visibly labelled as derived, every
 * line carrying its evidence link. The identical brief object ships in
 * the JSON twin (/api/projects.json `briefs`). */
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

export function projectsBodyHtml(
  pool: ProjectPoolItem[],
  opts: {
    area?: string | null;
    orchestration?: OrchestrationSection | null;
    detail?: ProvenanceDetail;
    actionsByObject?: Map<string, DashAction[]>;
    identities?: AuthorIdentity[];
    coverage?: CoverageSnapshotFile | null;
    /** Where the area chips point: /projects standalone, /program once merged (#1497). */
    basePath?: string;
    /** Prebuilt briefs (#1279), e.g. with comment activity folded in.
     * Absent: briefs are built here from the pool + orchestration only. */
    briefs?: Map<RefKey, ItemBrief>;
  } = {},
): string {
  const base = opts.basePath ?? "/projects";
  const areas = [...new Set(pool.map((p) => p.area).filter((a): a is string => Boolean(a)))].sort();
  // #1143: Area-unset items get an explicit "(unset)" bucket, mirroring
  // the /program index banner — without it they are silently mixed into
  // "all" with no count and no way to view them as the gap they are.
  const UNSET_AREA = "(unset)";
  const unsetCount = pool.filter((p) => !p.area).length;
  const area =
    opts.area === UNSET_AREA && unsetCount > 0 ? UNSET_AREA : opts.area && areas.includes(opts.area) ? opts.area : null;
  const shown = area === UNSET_AREA ? pool.filter((p) => !p.area) : area ? pool.filter((p) => p.area === area) : pool;
  const chips = [
    `<a href="${base}"${!area ? ' class="active"' : ""}>all (${pool.length})</a>`,
    ...areas.map(
      (a) =>
        `<a href="${base}?area=${encodeURIComponent(a)}"${a === area ? ' class="active"' : ""}>${esc(a)} (${pool.filter((p) => p.area === a).length})</a>`,
    ),
    ...(unsetCount > 0
      ? [
          `<a href="${base}?area=${encodeURIComponent(UNSET_AREA)}"${area === UNSET_AREA ? ' class="active"' : ""}>${UNSET_AREA} (${unsetCount})</a>`,
        ]
      : []),
  ].join("");
  if (!pool.length) {
    return `<div class="banner">No Project pool in the current fleet feed — the collector embeds it from the org Project <a href="${PROJECT_URL}">Developing Today (#4)</a> when a Projects token is available on the sandbox. Items on the board/roadmap fall back to the projector/fixtures chain until the pool lands.</div>${coverageSectionHtml(opts.coverage)}<p class="muted">Live services: ${serviceHomeLinks()
      .map((s) => `<a href="${esc(s.url)}">${esc(s.label)} ↗</a>`)
      .join(" · ")}</p>`;
  }
  const areaWeights = areas
    .map((a) => {
      const items = pool.filter((p) => p.area === a);
      const w = summarizeWeights(items);
      return `<div class="card"><strong>${esc(a)}</strong> — ${esc(weightedLine(w))}<br><span class="muted">${esc(handsLine(w))}</span></div>`;
    })
    .join("");
  const shownWeights = summarizeWeights(shown);
  const epicRollups = buildEpicRollups(pool);
  const cards = shown
    .map((p) => {
      const derived = derivedProjectState(p, opts.orchestration);
      const clash = projectDisagreement(p, opts.orchestration);
      const clashBadge = clash
        ? ` <span class="badge stale" title="The Project Status and the state the rest of the dashboard derives disagree — both are shown, neither is preferred">⚠ project: ${esc(clash.project)} · derived: ${esc(clash.derived)}</span>`
        : "";
      const statusBadge = p.status
        ? `<span class="badge ${p.status === "Done" ? "merged" : p.status === "Todo" ? "closed" : "implementing"}">${esc(p.status)}</span>`
        : `<span class="badge">no status</span>`;
      const meta = [
        p.area ? `<span class="badge src">${esc(p.area)}</span>` : "",
        p.horizon ? `<span class="badge src">${esc(p.horizon)}</span>` : "",
        p.kind ? `<span class="badge src">${esc(p.kind)}</span>` : "",
        `<span class="badge ${p.state === "open" ? "open" : "closed"}">issue ${esc(p.state)}</span>`,
        `<span class="badge src">derived: ${esc(derived)}</span>`,
      ].join(" ");
      // #1154: the status-request control + the store's visible state.
      const objectRef = ghObjectRef(p.ref);
      const statusCtl = opts.identities
        ? `${actionBadgesHtml(opts.actionsByObject?.get(objectRef))}${statusRequestHtml(objectRef, p.ref, p.status ?? null, { identities: opts.identities })}`
        : "";
      const rollup = p.kind === "Epic" ? epicRollups.get(p.ref) : undefined;
      const epicRollup = rollup ? epicRollupHtml(rollup) : "";
      const brief = opts.briefs?.get(p.ref) ?? buildItemBrief(p, { pool, orchestration: opts.orchestration });
      return `<div class="card"><strong>${esc(p.title)}</strong>${clashBadge}<br>${statusBadge} ${meta}<br><span class="muted"><a href="${esc(p.issueUrl)}">${esc(p.repo)}#${p.number}</a> · <a href="${esc(p.url)}">Project item</a> · <a href="${esc(p.projectUrl)}">Project #4</a>${p.updatedAt ? ` · updated ${fmtDate(p.updatedAt)}` : ""}</span>${epicRollup}${statusCtl}${briefPanelHtml(brief)}</div>`;
    })
    .join("");
  const body = `<p class="muted">${pool.length} items from the org Project <a href="${PROJECT_URL}">Developing Today (#4)</a>, read live by the fleet-feed collector (GraphQL, read-only) and carried in the feed. This pool is the resolution source for board/roadmap GitHub items. Where the Project Status and the derived state disagree, the card shows ⚠ with both values. Machine-first: <a href="/api/projects.json">/api/projects.json</a>.</p>
<div class="chips">${chips}</div>
${unsetCount > 0 ? `<div class="banner">${unsetCount} pool item${unsetCount === 1 ? "" : "s"} carry no Area — they belong to no project page until Area is set on the Project. Setting Area is the orchestrator's reconciliation job; this count keeps the gap visible. <a href="${base}?area=${encodeURIComponent(UNSET_AREA)}">View the (unset) bucket →</a></div>` : ""}
${coverageSectionHtml(opts.coverage)}
<p class="muted">Weighted progress (effort xs=1 · s=4 · m=12 · l=40, #1108): <strong>${esc(weightedLine(shownWeights))}</strong><br>${esc(handsLine(shownWeights))}</p>
${area ? "" : `<h2>Per-Area weighted progress</h2>${areaWeights}`}
<p class="muted">Live services: ${serviceHomeLinks()
    .map((s) => `<a href="${esc(s.url)}">${esc(s.label)} ↗</a>`)
    .join(" · ")}</p>
${cards || '<p class="muted">No items in this area.</p>'}`;
  return body;
}

export function renderProjects(
  pool: ProjectPoolItem[],
  provenance: Provenance,
  opts: {
    area?: string | null;
    orchestration?: OrchestrationSection | null;
    detail?: ProvenanceDetail;
    actionsByObject?: Map<string, DashAction[]>;
    identities?: AuthorIdentity[];
    coverage?: CoverageSnapshotFile | null;
    basePath?: string;
  } = {},
): string {
  return layout(
    "Projects",
    `<h1>Projects — GitHub Project pool</h1>
${projectsBodyHtml(pool, opts)}`,
    provenance,
    "/projects",
    opts.identities ? ACTION_SCRIPT : "",
    opts.detail ? freshnessChips(opts.detail, ["items"]) : "",
  );
}

// --- Program status page (#1046: the dashboard documents itself) ------------
// Editorial sections come from src/program.ts (the PROGRAM object — the
// one file to edit when the program changes); the live-status section is
// derived from the Source at request time by buildProgramLive, so its
// claims cannot drift from the tracked issues / Project pool /
// orchestrator they describe. Unresolvable refs render as "unresolved".

function programEntryHtml(e: ProgramEntry, items: Map<RefKey, GhItem>): string {
  const refs = (e.refs ?? [])
    .map((r) => {
      const item = items.get(r);
      return item
        ? itemBadge(item)
        : `<span class="badge closed" title="The current source cannot resolve this ref">unresolved ${esc(r)}</span>`;
    })
    .join(" ");
  const links = (e.links ?? []).map((l) => `<a href="${esc(l.url)}">${esc(l.label)}</a>`).join(" · ");
  const meta = [refs, links].filter(Boolean).join(" ");
  return `<div class="card"><strong>${esc(e.title)}</strong><br>${esc(e.detail)}${meta ? `<br>${meta}` : ""}</div>`;
}

function programEntries(entries: ProgramEntry[], items: Map<RefKey, GhItem>): string {
  return entries.map((e) => programEntryHtml(e, items)).join("");
}

function countList(list: { name: string; count: number }[]): string {
  return list.length ? list.map((c) => `${esc(c.name)}: ${c.count}`).join(" · ") : '<span class="muted">none</span>';
}

function trackedTable(tracked: TrackedIssue[]): string {
  const rows = tracked
    .map(({ ref, item }) =>
      item
        ? `<tr><td>${itemBadge(item)}</td><td>${esc(item.title)}</td><td><span class="badge ${item.state === "open" ? "open" : item.state === "merged" ? "merged" : "closed"}">${esc(item.state)}</span></td><td>${fmtDate(item.updatedAt)}</td></tr>`
        : `<tr><td><span class="badge closed">unresolved</span></td><td><code>${esc(ref)}</code></td><td><span class="muted">unresolved — the current source cannot resolve this ref</span></td><td>—</td></tr>`,
    )
    .join("");
  return `<div class="tblwrap"><table class="table table-zebra"><caption>Tracked issues</caption><tr><th scope="col">Tracked issue</th><th scope="col">Title</th><th scope="col">State</th><th scope="col">Updated</th></tr>${rows}</table></div>`;
}

function poolSummaryHtml(live: ProgramLive): string {
  return live.pool.total
    ? `<div class="card"><strong><a href="/program#projects-pool">${live.pool.total} items</a></strong> in the org Project pool<br><span class="muted">By status — ${countList(live.pool.byStatus)}</span><br><span class="muted">By area — ${countList(live.pool.byArea)}</span></div>`
    : `<div class="banner">No Project pool in the current feed — the collector embeds it from the org Project when a Projects token is available. Pool summaries appear here when it lands; tracked issues still resolve through the projector/fixtures chain.</div>`;
}

function orchestrationHtml(live: ProgramLive): string {
  return live.orchestration
    ? `<div class="card"><strong>Orchestrator</strong> — latest run <code>${esc(live.orchestration.runId)}</code>, published ${fmtDate(live.orchestration.generatedAt)}.<br><span class="muted">The tiers and goal measurements it publishes are on <a href="/attention">Needs you</a> and <a href="/today">Today</a>.</span></div>`
    : `<div class="card"><strong>Orchestrator</strong> — <span class="muted">no orchestration section in the current feed yet (not measured). The Today view derives and labels itself until one lands.</span></div>`;
}

function projectCard(s: ProjectSummary): string {
  const poolLine =
    s.poolArea === null
      ? `<span class="muted">No dedicated Project Area yet.</span>`
      : `<strong><a href="/program?area=${encodeURIComponent(s.poolArea)}">${s.poolTotal} pool items</a></strong> <span class="muted">(${esc(s.poolArea)})</span>${s.poolTotal ? `<br><span class="muted">Weighted — ${esc(weightedLine(s.weights))}</span><br><span class="muted">${esc(handsLine(s.weights))}</span><br><span class="muted">By status — ${countList(s.poolByStatus)}</span>` : ""}`;
  const roadmapLine =
    s.roadmapArea === null
      ? `<span class="muted">No Grist roadmap area yet.</span>`
      : `<span class="muted">${s.roadmapRows} roadmap row${s.roadmapRows === 1 ? "" : "s"} (${esc(s.roadmapArea)})</span>`;
  const homeLine = s.homeDoc
    ? `<br><span class="muted">Home doc: <a href="${esc(s.homeDoc.url)}">${esc(s.homeDoc.path)}</a></span>`
    : "";
  return `<div class="card"><strong><a href="${esc(s.route)}">${esc(s.title)}</a></strong><br>${poolLine}<br>${roadmapLine}${homeLine}<br><a href="${esc(s.route)}">Project page →</a></div>`;
}

/** /program as the projects index (#1121): one card per configured
 * project from the registry, the whole-pool + orchestrator live blocks
 * from #1046, and the unset-Area count as a visible gap. The full
 * pm-dashboard document moved to /program/pm-dashboard — every link
 * that pointed at /program still lands here. */
export function renderProgram(
  data: ProgramData,
  items: Map<RefKey, GhItem>,
  provenance: Provenance,
  detail?: ProvenanceDetail,
  poolSection?: {
    pool: ProjectPoolItem[];
    area?: string | null;
    coverage?: CoverageSnapshotFile | null;
    orchestration?: OrchestrationSection | null;
    actionsByObject?: Map<string, DashAction[]>;
    identities?: AuthorIdentity[];
    /** Prebuilt briefs (#1279), e.g. with comment activity folded in. */
    briefs?: Map<RefKey, ItemBrief>;
  } | null,
): string {
  const poolSectionHtml = poolSection
    ? `<h2 id="projects-pool">Projects pool</h2>
<p class="muted">The Projects page, merged into Program status by the #1497 curation — the raw org Project pool with its Project-vs-derived disagreements. <code>/projects</code> redirects to this section; the machine twin <a href="/api/projects.json">/api/projects.json</a> is unchanged.</p>
${projectsBodyHtml(poolSection.pool, { area: poolSection.area, orchestration: poolSection.orchestration, actionsByObject: poolSection.actionsByObject, identities: poolSection.identities, coverage: poolSection.coverage, basePath: "/program", briefs: poolSection.briefs })}`
    : "";
  const { program, live } = data;
  const projects = data.projects ?? [];
  const unset = live.pool.byArea.find((c) => c.name === "(unset)")?.count ?? 0;
  const cards = projects.map(projectCard).join("");
  const unsetHtml =
    live.pool.total && unset
      ? `<div class="banner">${unset} pool item${unset === 1 ? "" : "s"} carry no Area — they belong to no project page until Area is set on the Project. Setting Area is the orchestrator's reconciliation job; this count keeps the gap visible.</div>`
      : "";
  const body = `<h1>Program status — projects</h1>
<p class="muted">Every project the fleet runs, documented the same way: a description, its live Project-pool and roadmap sections, its tracked issues resolved live, and its roadmap — all driven by the project registry in <code>src/project-config.ts</code>. Pick a project for its full page.</p>
<p class="muted">Source of truth: the <a href="${esc(program.designRecordUrl)}">design record</a> in developing-today/code · the org Project <a href="${esc(program.projectUrl)}">Developing Today (#4)</a> · Machine-first: <a href="/api/program.json">/api/program.json</a>.</p>
<p class="muted">What runs where — every stack service, its port and bind, and its live health: <a href="/health#topology">Health &amp; topology →</a></p>
<h2>Projects</h2>
<div class="board">${cards || '<p class="muted">No projects configured.</p>'}</div>
${unsetHtml}
<h2>Live status</h2>
<p class="muted">Derived at request time from the Project pool and the latest orchestration section — not written by hand.</p>
${poolSummaryHtml(live)}
${orchestrationHtml(live)}
<h2>The PM dashboard's own record</h2>
<p class="muted">This dashboard's architecture, what's built, what's next, and its long-term roadmap live on its project page: <a href="/program/pm-dashboard">PM dashboard →</a>. Its tracked issues, resolved live:</p>
${trackedTable(live.tracked)}${poolSectionHtml}`;
  return layout(
    "Program status",
    body,
    provenance,
    "/program",
    poolSection?.identities ? ACTION_SCRIPT : "",
    detail ? freshnessChips(detail, ["orchestration", "items", "roadmap"]) : "",
  );
}

// --- Per-project page (#1121, registry in src/project-config.ts) -------------

const POOL_ITEM_CAP = 24;

/** Render a graph ref as a link only when it satisfies the shared ref rule. */
function poolRefLink(ref: RefKey): string {
  const label = esc(ref);
  return isValidRef(ref) ? `<a href="${esc(refUrl(ref))}">${label}</a>` : label;
}

/** The #2400 graph section of a project page: blocked weight + longest
 * blocked chain and the depth-graph SVG, from the part-1 edge contract
 * (#2399). Graceful absence is explicit: a pool with no effort labels
 * and no edges says "not yet sized/graphed" (never a fake 0%); a sized
 * pool without edges says "not yet graphed" only. */
function poolGraphHtml(items: ProjectPoolItem[], allW: ReturnType<typeof summarizeWeights>): string {
  const edges = hasGraphEdges(items);
  if (!edges && allW.totalWeight === 0) {
    return `<div class="banner">Not yet sized/graphed: no pool item for this project carries an effort/* label or an issue graph edge yet, so there is no weighted or graph view to draw. The counts above are raw item counts — not a 0% weighted score.</div>`;
  }
  if (!edges) {
    return `<p class="muted">Not yet graphed: pool items carry no parent / blocked-by edges yet, so blocked weight and the depth graph appear once the collector sees edges. Weighted progress above is from effort labels alone.</p>`;
  }
  const blocked = blockedSummary(items);
  const chainHtml =
    blocked.longestChain.length >= 2
      ? ` · longest blocked chain: ${blocked.longestChain.map(poolRefLink).join(" → ")} (${blocked.longestChain.length - 1} link${blocked.longestChain.length === 2 ? "" : "s"})`
      : "";
  const blockedHtml = blocked.blockedCount
    ? `<p class="muted"><strong>Blocked weight: ${blocked.blockedWeight}</strong> across ${blocked.blockedCount} open item${blocked.blockedCount === 1 ? "" : "s"} waiting on an unresolved blocker${blocked.blockedUnsizedCount ? ` (${blocked.blockedUnsizedCount} of them unsized — counted, never zero-weighted)` : ""}${chainHtml}.</p>`
    : `<p class="muted"><strong>Blocked weight: 0</strong> — no open item is waiting on an unresolved blocker.</p>`;

  const layout = depthLayout(items);
  const pos = new Map(layout.nodes.map((n) => [n.ref, n]));
  const X = (depth: number) => 16 + depth * 180;
  const Y = (row: number) => 16 + row * 26;
  const width = X(layout.maxDepth) + 340;
  const height = Y(layout.nodes.length - 1) + 16;
  const edgeLines = layout.edges
    .map((e) => {
      const a = pos.get(e.from)!;
      const b = pos.get(e.to)!;
      return `<line x1="${X(a.depth)}" y1="${Y(a.row)}" x2="${X(b.depth)}" y2="${Y(b.row)}" stroke="#8b949e" stroke-width="1"/>`;
    })
    .join("");
  const nodeShapes = layout.nodes
    .map((n) => {
      const label = `#${n.number} ${n.title.length > 34 ? `${n.title.slice(0, 33)}…` : n.title}`;
      const dash = n.mode === "hybrid" ? ` stroke-dasharray="3 2"` : "";
      const strokeW = n.mode === "physical" ? 3 : 1.5;
      const shape = n.epic
        ? `<rect x="${X(n.depth) - 6}" y="${Y(n.row) - 6}" width="12" height="12" fill="${n.state === "open" ? "#8957e5" : "#6e7781"}" stroke="#0d1117" stroke-width="${strokeW}"${dash}/>`
        : `<circle cx="${X(n.depth)}" cy="${Y(n.row)}" r="6" fill="${n.state === "open" ? "#bf8700" : "#6e7781"}" stroke="#0d1117" stroke-width="${strokeW}"${dash}/>`;
      return `${shape}<a href="${esc(n.issueUrl)}"><text x="${X(n.depth) + 11}" y="${Y(n.row) + 4}" font-size="11" fill="currentColor">${esc(label)}</text></a>`;
    })
    .join("");
  const truncated = layout.truncated
    ? `<p class="muted">Depth graph shows ${layout.nodes.length} of ${items.length} items (cap) — the full set is in the cards below.</p>`
    : "";
  const svg = layout.nodes.length
    ? `<svg viewBox="0 0 ${width} ${height}" width="100%" role="img" aria-label="Issue depth graph" style="max-width:100%;height:auto">${edgeLines}${nodeShapes}</svg><p class="muted">Depth = sub-issue nesting from the Project pool. Squares are epics; amber = open, grey = closed. Border: thick solid = mode/physical, dashed = mode/hybrid, thin = online or no mode.</p>${truncated}`
    : "";
  return `<h3>Blocked + depth graph</h3>${blockedHtml}${svg}`;
}

/** The #3973 per-epic dependency graph section of a project page
 * (design doc/2026-11-01T21-05-00Z_design_aggregator_dependency_graph_1257.md):
 * an epic selector with item/blocked counts, then per epic a summary +
 * ranked blocked list beside the graph SVG and a vertical,
 * keyboard-readable dependency list. The list is the primary surface
 * on narrow screens (the flex column stacks it full-width; the SVG
 * may scroll horizontally as a secondary detail). Dependency edges
 * are dashed amber, hierarchy edges solid grey, and every node's
 * state is written as text — color is never the only signal. Empty
 * or unavailable graph data says so; it is never an empty success. */
/** Section anchor for one epic's graph, derived from the canonical ref
 * (owner/repo#number), never the bare issue number: two repositories
 * can hold epics with the same number, and bare-number ids collide. */
function epicGraphAnchor(ref: RefKey): string {
  return `epic-graph-${ref.replace(/[^a-zA-Z0-9]+/g, "-")}`;
}

function epicGraphSectionHtml(items: ProjectPoolItem[], selectorItems: ProjectPoolItem[] = items): string {
  // Selector entries and rendered sections are scoped to the project's
  // own pool (selectorItems = detail.poolItems): on a normal project
  // request the caller passes the entire org pool as `items` for
  // membership and blocker resolution, and rendering every eligible
  // epic from every Area would bury this project's section in unrelated
  // projects (Codex round-2 on #3973). Resolution inside each section
  // still runs against the full `items` pool.
  // The selector rollups are computed from the full `items` pool too,
  // then filtered to this project's epic refs (Codex round-3 on
  // #3973): computing them from the scoped pool made a member whose
  // blocker lives in another Area count as blocked in the selector
  // while its section — resolving against the full pool — correctly
  // reported it unblocked.
  const projectEpicRefs = new Set(selectorItems.filter((i) => i.kind === "Epic").map((i) => i.ref));
  const selector = epicSelector(items).filter((e) => projectEpicRefs.has(e.ref));
  if (!selector.length) {
    return `<h3>Per-epic dependency graph</h3><div class="banner">Dependency graph unavailable: no epic in this pool has items yet, so there is no per-epic graph to draw — this is an unavailable state, not an empty graph.</div>`;
  }
  const selectorHtml = `<nav class="epic-graph-selector" aria-label="Epic graph selector"><ul class="epic-graph-list">${selector
    .map(
      (e) =>
        `<li><a href="#${epicGraphAnchor(e.ref)}">${esc(e.title)}</a> <span class="muted">${esc(e.ref)} · ${e.itemCount} item${e.itemCount === 1 ? "" : "s"} · ${e.blockedCount} blocked · ${e.doneCount} done</span></li>`,
    )
    .join("")}</ul></nav>`;
  const sections = selector.map((entry) => epicGraphOneHtml(items, entry)).join("");
  return `<h3>Per-epic dependency graph</h3>${selectorHtml}${sections}`;
}

function epicGraphOneHtml(items: ProjectPoolItem[], entry: ReturnType<typeof epicSelector>[number]): string {
  const g = epicGraph(items, entry.ref);
  const heading = `<h4 id="${epicGraphAnchor(entry.ref)}">${esc(entry.title)} <span class="muted">(${esc(entry.ref)})</span></h4>`;
  if (g.status === "unavailable" || !g.epic) {
    return `<section>${heading}<div class="banner">Dependency graph unavailable: ${esc(g.reason ?? "no graph data for this epic")}.</div></section>`;
  }
  const poolByRef = new Map(items.map((p) => [p.ref, p]));
  const unresolvedOf = (ref: RefKey): boolean => {
    const held = poolByRef.get(ref);
    return held === undefined || held.state === "open";
  };
  const summary = `<p class="muted"><strong>${g.memberCount} item${g.memberCount === 1 ? "" : "s"} in this epic</strong> · ${g.blockedCount} blocked on an unresolved blocker · ${g.externalCount} one-hop reference${g.externalCount === 1 ? "" : "s"} outside this pool (unresolved — status unknown, never counted as resolved).</p>`;
  const truncatedHtml = g.truncated
    ? `<p class="muted">Truncated: this graph shows ${g.nodes.length} of ${g.totalNodes} nodes (cap ${EPIC_GRAPH_DEFAULT_CAP}). The counts above cover the full set; a capped graph is not complete.</p>`
    : "";

  // Ranked summary: open member items by how many distinct open epic items
  // are in the transitive blockedBy ancestry of their blockers (the full
  // transitive downstream ranking is #3974). Never derived from the
  // capped node list: a member the cap hid is still waiting.
  const ranked = g.rankedBlocked;
  const rankedHtml = ranked.length
    ? `<ul class="epic-graph-list">${ranked
        .map(
          (r) => {
            const blockerItems = r.blockers.map((b) => poolRefLink(b));
            const contributingItems = r.contributingItems.length > 0
              ? `<br><span class="muted">Contributing downstream epics: ${r.contributingItems.map((ci) => poolRefLink(ci)).join(", ")}</span>`
              : "";
            return `<li><a href="${esc(r.issueUrl)}">${esc(r.ref)}</a> — waiting on ${r.downstreamCount} distinct downstream open epic item${r.downstreamCount === 1 ? "" : "s"} via ${r.blockers.length} unresolved blocker${r.blockers.length === 1 ? "" : "s"}: ${blockerItems.join(", ")}${contributingItems}</li>`;
          },
        )
        .join("")}</ul>`
    : `<p class="muted">No open item in this epic is waiting on an unresolved blocker.</p>`;

  // SVG: hierarchy edges solid grey, dependency edges dashed amber —
  // distinct by dash and color, and named in the legend text.
  const pos = new Map(g.nodes.map((n) => [n.ref, n]));
  const X = (depth: number) => 16 + depth * 180;
  const Y = (row: number) => 16 + row * 26;
  const width = X(g.maxDepth) + 340;
  const height = g.nodes.length ? Y(g.nodes.length - 1) + 16 : 32;
  // Dependency edges carry an arrowhead marker pointing at the blocker
  // (the `to` node): hierarchy depth alone does not show which sibling
  // blocks which (Codex round-2 on #3973). The marker id is per-epic --
  // several epic sections share one page, and duplicate ids would make
  // every edge reference the first section's marker.
  const depMarkerId = `dep-arrow-${epicGraphAnchor(g.epic.ref)}`;
  const depDefs = g.edges.some((e) => e.type === "dependency")
    ? `<defs><marker id="${depMarkerId}" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" fill="#d29922"/></marker></defs>`
    : "";
  const edgeLines = g.edges
    .map((e) => {
      const a = pos.get(e.from)!;
      const b = pos.get(e.to)!;
      return e.type === "dependency"
        ? `<line x1="${X(a.depth)}" y1="${Y(a.row)}" x2="${X(b.depth)}" y2="${Y(b.row)}" stroke="#d29922" stroke-width="1.5" stroke-dasharray="5 3" marker-end="url(#${depMarkerId})"/>`
        : `<line x1="${X(a.depth)}" y1="${Y(a.row)}" x2="${X(b.depth)}" y2="${Y(b.row)}" stroke="#8b949e" stroke-width="1"/>`;
    })
    .join("");
  const nodeShapes = g.nodes
    .map((n) => {
      const label = n.title
        ? `#${n.number} ${n.title.length > 34 ? `${n.title.slice(0, 33)}…` : n.title}`
        : `${n.ref} (outside this pool)`;
      const x = X(n.depth);
      const y = Y(n.row);
      const shape = n.unresolved
        ? `<polygon points="${x},${y - 7} ${x + 7},${y} ${x},${y + 7} ${x - 7},${y}" fill="none" stroke="#d29922" stroke-width="1.5" stroke-dasharray="3 2"/>`
        : n.epic
          ? `<rect x="${x - 6}" y="${y - 6}" width="12" height="12" fill="${n.state === "open" ? "#8957e5" : "#6e7781"}" stroke="#0d1117" stroke-width="1.5"/>`
          : `<circle cx="${x}" cy="${y}" r="6" fill="${n.state === "open" ? "#bf8700" : "#6e7781"}" stroke="#0d1117" stroke-width="1.5"/>`;
      const nodeLabel = `<text x="${x + 11}" y="${y + 4}" font-size="11" fill="currentColor">${esc(label)}</text>`;
      return `${shape}${isValidRef(n.ref) ? `<a href="${esc(n.issueUrl)}">${nodeLabel}</a>` : nodeLabel}`;
    })
    .join("");
  const svg = g.edges.length
    ? `<div class="epic-graph-svg"><svg viewBox="0 0 ${width} ${height}" width="100%" role="img" aria-label="Dependency graph for epic ${esc(g.epic.ref)}" style="max-width:100%;height:auto">${depDefs}${edgeLines}${nodeShapes}</svg><p class="muted">Solid grey lines are sub-issue hierarchy (parent → child); dashed amber lines are dependency edges, with an arrowhead pointing at the blocker (item → blocker). Squares are epics; diamonds are references outside this pool whose status is unknown. Every node's state is also written as text in the list — color is never the only signal.</p></div>`
    : `<p class="muted">Not yet graphed: this epic's items carry no parent / blocked-by edges yet, so there is no graph to draw. The items are listed below.</p>`;

  const listHtml = `<ul class="epic-graph-list">${g.nodes
    .map((n) => {
      const stateText = n.unresolved
        ? "not in this pool / status unknown — unresolved"
        : `issue ${esc(n.state ?? "unknown")}`;
      const blockedBy = n.blockers.length
        ? `blocked by: ${n.blockers.map((b) => `${poolRefLink(b)}${unresolvedOf(b) ? " (unresolved)" : " (resolved)"}`).join(", ")}`
        : "blocked by: none observed";
      const blocks = n.dependents.length
        ? `blocks: ${n.dependents.map(poolRefLink).join(", ")}`
        : "blocks: none observed";
      const name = n.title ? `${esc(n.title)}` : "(outside this pool — no title in this feed)";
      const nodeRef = isValidRef(n.ref) ? `<a href="${esc(n.issueUrl)}">${esc(n.ref)}</a>` : esc(n.ref);
      return `<li>${nodeRef} ${name} — ${stateText}; ${blockedBy}; ${blocks}</li>`;
    })
    .join("")}</ul>`;

  return `<section>${heading}${summary}${truncatedHtml}<div class="epic-graph-flex">${svg}<div class="epic-graph-side"><h5>Blocked in this epic</h5>${rankedHtml}<h5>All items — dependency list</h5>${listHtml}</div></div></section>`;
}

export function renderProgramProject(
  detail: ProjectDetail,
  items: Map<RefKey, GhItem>,
  provenance: Provenance,
  provDetail?: ProvenanceDetail,
  epicPool: ProjectPoolItem[] = detail.poolItems,
): string {
  const { config } = detail;
  const linkBits: string[] = [];
  if (config.homeDoc) {
    linkBits.push(
      `Home doc: <a href="${esc(config.homeDoc.url)}">${esc(config.homeDoc.path)}</a> <span class="muted">(${esc(config.homeDoc.repo)})</span>`,
    );
  } else {
    linkBits.push(`<span class="muted">No home doc configured for this project yet.</span>`);
  }
  if (config.statusReport) {
    linkBits.push(`Status report: <a href="${esc(config.statusReport.url)}">${esc(config.statusReport.label)}</a>`);
  } else {
    linkBits.push(`<span class="muted">No status report yet for this project.</span>`);
  }
  for (const l of config.dashboardLinks ?? []) {
    linkBits.push(
      `<a href="${esc(l.route)}">${esc(l.label)}</a>${l.note ? ` <span class="muted">(${esc(l.note)})</span>` : ""}`,
    );
  }
  linkBits.push(`<a href="/api/program-project.json?id=${encodeURIComponent(config.id)}">JSON twin</a>`);

  const archHtml = config.architecture?.length
    ? `<h2>What this is — architecture</h2>${programEntries(config.architecture, items)}`
    : "";

  let poolHtml: string;
  if (config.poolArea === null) {
    poolHtml = `<div class="banner">No dedicated Project Area yet — this project's items are not in the org Project pool under their own Area. ${detail.unsetPoolCount} pool item${detail.unsetPoolCount === 1 ? "" : "s"} currently carry no Area at all. Its work is tracked in the tracked issues above and its home doc / links, where configured.</div>`;
  } else if (!detail.poolItems.length) {
    poolHtml = `<div class="banner">No pool items with Area <code>${esc(config.poolArea)}</code> in the current feed — the collector embeds the pool from the org Project when a Projects token is available, and reconciliation assigns Area.</div>`;
  } else {
    const shown = detail.poolItems.slice(0, POOL_ITEM_CAP);
    const cards = shown
      .map((p) => {
        const statusBadge = p.status
          ? `<span class="badge ${p.status === "Done" ? "merged" : p.status === "Todo" ? "closed" : "implementing"}">${esc(p.status)}</span>`
          : `<span class="badge">no status</span>`;
        const meta = [
          p.horizon ? `<span class="badge src">${esc(p.horizon)}</span>` : "",
          p.kind ? `<span class="badge src">${esc(p.kind)}</span>` : "",
          `<span class="badge ${p.state === "open" ? "open" : "closed"}">issue ${esc(p.state)}</span>`,
        ].join(" ");
        return `<div class="card"><strong>${esc(p.title)}</strong><br>${statusBadge} ${meta}<br><span class="muted"><a href="${esc(p.issueUrl)}">${esc(p.repo)}#${p.number}</a> · <a href="${esc(p.url)}">Project item</a>${p.updatedAt ? ` · updated ${fmtDate(p.updatedAt)}` : ""}</span></div>`;
      })
      .join("");
    const more =
      detail.poolItems.length > POOL_ITEM_CAP
        ? `<p class="muted">Showing ${POOL_ITEM_CAP} of ${detail.poolItems.length} — the full set is on <a href="/projects?area=${encodeURIComponent(config.poolArea)}">/projects</a>.</p>`
        : "";
    const allW = summarizeWeights(detail.poolItems);
    const epics = detail.poolItems.filter((p) => p.kind === "Epic");
    const epicW = summarizeWeights(epics);
    const epicRollups = buildEpicRollups(epicPool);
    const epicRows = epics
      .map((p) => {
        const w = effortWeight(p.labels);
        const epicRollup = epicRollups.get(p.ref);
        const rollup = epicRollup ? epicRollupHtml(epicRollup) : "";
        return `<div class="card"><strong>${esc(p.title)}</strong><br><span class="muted">weight: ${w === null ? "unsized" : w} · issue ${esc(p.state)}${p.status ? ` · status: ${esc(p.status)}` : ""} · <a href="${esc(p.issueUrl)}">${esc(p.repo)}#${p.number}</a></span>${rollup}</div>`;
      })
      .join("");
    const epicHtml = epics.length
      ? `<h3>Epics — weighted progress</h3><p class="muted"><strong>${esc(weightedLine(epicW))}</strong></p>${epicRows}`
      : "";
    poolHtml = `<p class="muted"><strong>${esc(weightedLine(allW))}</strong><br>${esc(handsLine(allW))}<br><strong>${detail.poolItems.length} items</strong> with Area <code>${esc(config.poolArea)}</code> — by status: ${countList(detail.poolByStatus)}. Full pool view: <a href="/projects?area=${encodeURIComponent(config.poolArea)}">/projects?area=${esc(config.poolArea)}</a>.</p>${epicHtml}${poolGraphHtml(detail.poolItems, allW)}${epicGraphSectionHtml(epicPool, detail.poolItems)}${cards}${more}`;
  }

  let roadmapHtml: string;
  if (config.roadmapArea === null) {
    roadmapHtml = `<div class="banner">No Grist roadmap area yet — this project has no rows in the Program roadmap table. Its horizons live in its home doc and tracked issues.</div>`;
  } else if (!detail.roadmapRows.length) {
    roadmapHtml = `<div class="banner">No roadmap rows with area <code>${esc(config.roadmapArea)}</code> in the current source.</div>`;
  } else {
    const rows = detail.roadmapRows
      .map((r) => {
        const links = r.links
          .map((l) => `<a class="badge src" href="${esc(refUrl(l.ref))}">${esc(l.ref)}</a>`)
          .join(" ");
        return `<div class="card"><strong>${esc(r.title)}</strong><br><span class="badge src">${esc(r.status)}</span> <span class="badge src">${esc(r.horizon)}</span>${links ? ` ${links}` : ""}${r.notes ? `<br><span class="muted">${esc(r.notes)}</span>` : ""}<br><span class="muted"><a href="/discuss/${encodeURIComponent(r.subject)}">💬 comments</a> · <a href="/roadmap">roadmap</a></span></div>`;
      })
      .join("");
    roadmapHtml = `<p class="muted">${detail.roadmapRows.length} row${detail.roadmapRows.length === 1 ? "" : "s"} with area <code>${esc(config.roadmapArea)}</code> in Grist's Program roadmap.</p>${rows}`;
  }

  const repoCardsHtml = detail.repoCards.length
    ? `<h2>Per-repo cards</h2>
<p class="muted">One card per working repo. Open-issue counts are a dated GitHub-search snapshot (date on each card), never a live claim; the pool items and roadmap rows are split live from the same sections above, by repo.</p>${detail.repoCards
        .map(({ card, poolItems, roadmapRows, tracked }) => {
          const refs = tracked
            .map(
              ({ ref, item }) =>
                `<a class="badge src" href="${esc(refUrl(ref))}">${esc(ref)}</a>${item ? ` <span class="badge ${item.state === "open" ? "open" : "closed"}">${esc(item.state)}</span>` : ""}`,
            )
            .join(" ");
          return `<div class="card"><strong><a href="https://github.com/${esc(card.repo)}">${esc(card.repo)}</a></strong><br>${esc(card.role)}<br><span class="muted">Open issues (GitHub search, ${esc(card.openIssuesAsOf)}): <strong>${card.openIssues}</strong> · fleet pool items in this repo: ${poolItems.length} · roadmap rows: ${roadmapRows.length}</span>${refs ? `<br>${refs}` : ""}</div>`;
        })
        .join("")}`
    : "";

  const sweepHealthHtml = config.sweepHealth?.length
    ? `<h2>Sweep health — published fleet state</h2>
<p class="muted">Built only from already-published fleet data — the orchestration section this aggregator serves and the fleet's tracked issues. No new data pipeline feeds this section.</p>${programEntries(config.sweepHealth, items)}`
    : "";

  const ed = config.editorial;
  const editorialHtml = ed
    ? `${ed.done.length ? `<h2>What's built</h2>${programEntries(ed.done, items)}` : ""}${ed.next.length ? `<h2>What's next</h2>${programEntries(ed.next, items)}` : ""}${ed.longTerm.length ? `<h2>Long-term roadmap</h2>${programEntries(ed.longTerm, items)}` : ""}${ed.gaps.length ? `<h2>Known gaps</h2>${programEntries(ed.gaps, items)}` : ""}`
    : "";

  const body = `<h1>${esc(config.title)}</h1>
<p class="muted">${esc(config.intro)}</p>
<p class="muted">${linkBits.join(" · ")}</p>
<p class="muted"><a href="/program">← all projects</a></p>
${archHtml}
<h2>Live status — tracked issues</h2>
<p class="muted">Resolved at request time from the same Source as every other view. A ref the current source cannot resolve says so — never an invented state.</p>
${trackedTable(detail.tracked)}
<h2>Project pool — live</h2>
${poolHtml}
<h2>Roadmap — live</h2>
${roadmapHtml}
${repoCardsHtml}
${sweepHealthHtml}
${editorialHtml}`;
  return layout(
    config.title,
    body,
    provenance,
    "/program",
    "",
    provDetail ? freshnessChips(provDetail, ["orchestration", "items", "roadmap"]) : "",
  );
}

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
      `<h1>Alerts</h1><p class="muted">The alerts ledger (Supabase <code>alerts</code> schema) is unreachable from this process right now — intake, delivery, and ack are paused with it. The rest of the dashboard is unaffected.</p>`,
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
<p class="muted">The alert throat (#3676): every sentinel posts to one intake; this page is the ack surface. Critical incidents push immediately to subscribed browsers; an unacknowledged critical escalates once after ${view.ackWindowMin} minutes. Warnings are recorded here and in the ledger — no push in v1. Machine-first: <a href="/api/alerts.json">/api/alerts.json</a>.</p>
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
      `<h1>Notifications</h1><p class="muted">The notification send ledger (Supabase <code>alerts.notification_sends</code>) is unreachable from this process right now — the roll-up is paused with it. The rest of the dashboard is unaffected.</p>`,
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
            `<div class="card"><strong>Proposal: default class <code>${esc(c.class)}</code> off</strong> — acted-on ratio ${pct(c.ratio)} over the rolling window (${c.acted} of ${c.sent} sends acted on within 24h), under the 10% threshold.<br><span class="muted">Proposal only — this page never changes a preference or a default. Turning a class off by default is Drew's call, made on the Alerts page's subscription preferences.</span></div>`,
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
