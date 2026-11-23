// render.ts split (#13051): attention (needs-you) view and the needs-Drew census.
// render.ts re-exports every public name from here; callers keep importing "./render.ts".

import { type AttentionView, sessionChildren, sessionRoots } from "./derive.ts";
import type { DashAction } from "./actions.ts";
import { cardState, type CardState } from "./card-state.ts";
import { DELEGATION_DEFAULT_OBJECT_REF, NEEDS_ORDER_OBJECT_REF, type NeedsOrder } from "./tier-items.ts";
import type { CommentKindCounts } from "./comments.ts";
import { needsDrewWait, type NeedsDrewCensus, type NeedsDrewItem } from "./needs-drew.ts";
import { feedAgeText } from "./feed.ts";
import type { DashComment, Provenance, ProvenanceDetail, RefKey, SourceCoverage, TierItem } from "./model.ts";
import type { ItemBrief } from "./brief.ts";
import type { AuthorIdentity } from "./registry.ts";
import type { DecisionSetSummary } from "./decisions.ts";

import {
  esc,
  fmtDate,
  DASH_AFTER_JS,
  COMMENT_SCRIPT,
  FRAG_WATCHDOG_SCRIPT,
  ACTION_SCRIPT,
  freshnessChips,
  layout,
  sessionObjectRef,
  tierItemObjectRef,
  type CommentFormOpts,
  relTime,
  cardDomId,
  authorSelectHtml,
  aformHtml,
  actboxHtml,
  delegationDefaultHtml,
  coverageHtml,
  briefPanelsForRefs,
} from "./render-core.ts";
import { sessionCard, tierItemCard } from "./render-quota.ts";

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

/**
 * The live "Questions" section (#9354): the open decision sets from
 * the decision-set store, newest first, each linking to its own
 * token page via `pageUrlFor`. Pure HTML from already-computed
 * summaries — no I/O here, so tests render it directly.
 * summaries === null means the store could not be read: the section
 * says "unavailable" rather than rendering a confident zero (the
 * needsDrewCensusHtml pattern). summaries === undefined means the
 * caller predates the section and it is omitted entirely.
 * Fully-resolved sets are excluded here as well as by callers, so
 * the counts can never include a settled set.
 */
const QUESTIONS_STAMP = new Intl.DateTimeFormat("en-US", {
  timeZone: "America/Chicago",
  year: "numeric",
  month: "short",
  day: "numeric",
  hour: "numeric",
  minute: "2-digit",
  timeZoneName: "short",
});

export function questionsSectionHtml(
  summaries: DecisionSetSummary[] | null | undefined,
  pageUrlFor: (summary: DecisionSetSummary) => string,
): string {
  if (summaries === undefined) return "";
  const allSetsLink = `<p><a href="/decisions">All decision sets</a></p>`;
  const footer = `<p class="muted">Live from the decision-set store at page load.</p>`;
  if (summaries === null) {
    return `<section class="questions-section"><h2 id="questions">Questions</h2><p class="muted">Unavailable — the decision-set store could not be read, so open questions cannot be reported here.</p>${allSetsLink}${footer}</section>`;
  }
  const open = summaries.filter((s) => s.openQuestions > 0).sort((a, b) => b.createdAt.localeCompare(a.createdAt));
  if (!open.length) {
    return `<section class="questions-section"><h2 id="questions">Questions</h2><p class="muted">No open questions right now — the stack is clear.</p>${allSetsLink}${footer}</section>`;
  }
  const totalOpen = open.reduce((n, s) => n + s.openQuestions, 0);
  const rows = open
    .map((s) => {
      const stamp = Number.isFinite(Date.parse(s.createdAt))
        ? QUESTIONS_STAMP.format(new Date(s.createdAt))
        : s.createdAt;
      return `<div class="card"><a href="${esc(pageUrlFor(s))}"><strong>${esc(s.title)}</strong></a><br><span class="muted">${s.openQuestions} open question${s.openQuestions === 1 ? "" : "s"} · ${s.items} item${s.items === 1 ? "" : "s"} · ${s.questions} question${s.questions === 1 ? "" : "s"} · Raised ${esc(stamp)}</span></div>`;
    })
    .join("");
  return `<section class="questions-section"><h2 id="questions">Questions</h2><p class="muted">${open.length} open set${open.length === 1 ? "" : "s"} · ${totalOpen} open question${totalOpen === 1 ? "" : "s"}</p>${rows}${allSetsLink}${footer}</section>`;
}

export function renderAttention(
  view: AttentionView,
  coverage: SourceCoverage[],
  provenance: Provenance,
  counts?: Map<string, number | CommentKindCounts>,
  detail?: ProvenanceDetail,
  act?: AttentionAct,
  briefs?: Map<RefKey, ItemBrief>,
  census?: NeedsDrewCensus | null,
  questionsHtml = "",
): string {
  const sections = attentionSections(view, coverage, counts, act, briefs, census);
  return layout(
    "Needs you",
    `<h1>Needs you</h1><p class="muted">The NEEDS USER queue is an intake the orchestrator works first — it resolves what it can and escalates only what genuinely requires the user, each escalated item with its reason. Everything else the fleet can work is the orchestrator's own queue. Finished runs and stale items expire out (2 h after a run finishes, 24 h at the latest). Machine-first: <a href="/api/attention.json">/api/attention.json</a> · <a href="/api/coverage.json">/api/coverage.json</a>. Incident alerts (watchdog push + ack) live on <a href="/alerts">Alerts</a>.</p>
${sections.header}${sections.queues}${sections.sessions}${questionsHtml}`,
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
  // FDR-0013: only hands items and review requests are the owner's.
  // A choice an agent parked on the owner is fleet work, listed apart
  // so the page never presents it as waiting on him.
  const hands = census.items.filter((i) => needsDrewWait(i) === "hands");
  const review = census.items.filter((i) => needsDrewWait(i) === "review");
  const parked = census.items.filter((i) => needsDrewWait(i) === "agent-decides");
  const line = (i: NeedsDrewItem, note: string): string =>
    `<li><a href="${esc(i.url)}">${esc(i.ref)}</a> ${i.kind === "pr" ? "(PR) " : ""}${esc(i.title)} — ${i.needsLabels
      .map((l) => `<code>${esc(l)}</code>`)
      .join(" ")} — ${note} <span class="muted">(updated ${esc(i.updatedAt)})</span></li>`;
  const reasonOf = (i: NeedsDrewItem): string =>
    i.reason ? esc(i.reason) : "no reason recorded on the item — open it for context";
  const answered = census.complete ? "" : " in the sources that answered";
  const handsRows = hands.length
    ? `<ul>${hands
        .map((i) =>
          line(
            i,
            `<strong>${esc(i.hands?.kind ?? "hands")}</strong>: ${
              i.hands?.action ? esc(i.hands.action) : "the item states the one action"
            }`,
          ),
        )
        .join("")}</ul>`
    : `<p class="muted">No open item needs a person's hands right now${answered}.</p>`;
  const reviewRows = review.length
    ? `<ul>${review.map((i) => line(i, reasonOf(i))).join("")}</ul>`
    : `<p class="muted">No open item carries user/review right now${answered}.</p>`;
  const parkedRows = parked.length
    ? `<p class="muted">An agent parked each of these on the owner without a hands action. They do not wait on him: an agent decides, records the decision on the item and continues (FDR-0013). This count should fall to zero.</p><ul>${parked
        .map((i) => line(i, reasonOf(i)))
        .join("")}</ul>`
    : `<p class="muted">Nothing is parked on the owner by an agent${answered}.</p>`;
  const rows = `<h3 id="needs-drew-hands">Needs a person's hands (${hands.length})</h3>${handsRows}<h3 id="needs-drew-review">Review requested (${review.length})</h3>${reviewRows}<h3 id="needs-drew-parked">Parked on the owner by an agent — fleet work (${parked.length})</h3>${parkedRows}`;
  return `<h2 id="needs-drew-census">Needs Drew — GitHub label census (${hands.length + review.length})</h2><p class="muted">Every open issue and PR carrying <code>user/review</code> or <code>user/escalated</code> across the four repos (#1226), split by how it waits. The count in the heading is hands items and review requests only. The labels are the system of record; this view reads them and never writes them. Census taken ${esc(census.fetchedAt)}${census.complete ? "" : " — incomplete, see below"}. Machine form: <a href="/api/needs-drew.json">/api/needs-drew.json</a>.</p>${failureNote}${rows}`;
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
 *  a refresh re-patches the container with itself (attrs included).
 *
 *  `initial` marks the shell placeholder (#10605): it carries
 *  `data-init` so the first paint fetches the section with no
 *  freshness event. The `/frag/*` payload that replaces it (outer
 *  mode) must leave `initial` off — a re-initialising element would
 *  fetch itself again on every patch, i.e. loop.
 */
export function attentionFragment(
  section: "header" | "queues" | "sessions",
  innerHtml: string,
  query = "",
  options: { initial?: boolean } = {},
): string {
  const q = query ? `?${query}` : "";
  const init = options.initial ? ` data-init="@get('/frag/attention/${section}${q}')"` : "";
  return `<div id="attn-${section}" data-frag${init} data-on:dashrefresh__window="@get('/frag/attention/${section}${q}')">${innerHtml}</div>`;
}

/**
 * The Datastar shell for /attention (#1444): nav + headings paint
 * immediately; the three heavy sections stream in as fragments.
 * Without JS, a meta refresh lands on the full server render.
 */
export function renderAttentionShell(
  provenance: Provenance,
  detail?: ProvenanceDetail,
  refs?: string,
  questionsHtml = "",
): string {
  const refsQuery = refs ? `refs=${encodeURIComponent(refs)}` : "";
  const skel = (label: string) => `<div class="skel">Loading ${label}…</div>`;
  const body = `<h1>Needs you</h1><p class="muted">Loading the Needs User and Needs Orchestrator queues…</p><noscript><meta http-equiv="refresh" content="0; url=/attention?full=1"></noscript>
${attentionFragment("header", skel("queue status"), refsQuery, { initial: true })}
${attentionFragment("queues", skel("the queues"), refsQuery, { initial: true })}
${attentionFragment("sessions", skel("sessions"), refsQuery, { initial: true })}${questionsHtml}`;
  return layout(
    "Needs you",
    body,
    provenance,
    "/attention",
    ACTION_SCRIPT + COMMENT_SCRIPT + BULK_SCRIPT + FRAG_WATCHDOG_SCRIPT,
    detail ? freshnessChips(detail, ["orchestration", "attention", "registry"]) : "",
  );
}
