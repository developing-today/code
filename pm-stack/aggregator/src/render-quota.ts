// render.ts split (#13051): quota view, usage/billing cards, service detail and history charts.
// render.ts re-exports every public name from here; callers keep importing "./render.ts".

import { analyticsSectionHtml } from "./quota-analytics-view.ts";
import { freshness, projectPace, resetMarkers, sourceOf, STATE_LABELS, windowPercent } from "./derive.ts";
import type { DashAction } from "./actions.ts";
import { cardState } from "./card-state.ts";
import type { CommentKindCounts } from "./comments.ts";
import { providerDisplayNames } from "./provider-display.ts";
import {
  type ChargeRow,
  type DashComment,
  type PaygLine,
  type Provenance,
  type ProvenanceDetail,
  type QuotaView,
  type SaasService,
  type SnapshotRecord,
  type TieredEntry,
  type TierItem,
  type UsageMetric,
  type UsageView,
  SAAS_GROUP_LABELS,
  SESSION_SOURCE_LABELS,
  countedEstimate,
  isValidRef,
  refUrl,
} from "./model.ts";
import type { AuthorIdentity } from "./registry.ts";

import {
  esc,
  fmtDate,
  fmtCentral,
  freshnessChips,
  layout,
  sessionObjectRef,
  tierItemObjectRef,
  commentBadge,
  type CommentFormOpts,
  actionBadgesHtml,
  cardDomId,
  actionHistoryHtml,
  cardCommentHtml,
  triageActionsHtml,
  whyUserBadge,
  needsUserBodyHtml,
  needsUserFormsHtml,
  sessionActionsHtml,
} from "./render-core.ts";

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
        l.settledUsd != null
          ? `$${l.settledUsd.toFixed(2)} ${l.settledSource === "invoice" ? "invoiced" : "settled"}${
              l.estimate
                ? ` · estimates $${l.estimate.low.toFixed(2)}–$${l.estimate.high.toFixed(2)}`
                : l.accruedUsd != null
                  ? ` · $${l.accruedUsd.toFixed(2)} accrued`
                  : ""
            }${l.settledVarianceUsd != null ? ` · variance ${l.settledVarianceUsd < 0 ? "-$" : "+$"}${Math.abs(l.settledVarianceUsd).toFixed(2)}` : ""} · $${(l.unbilledUsd ?? 0).toFixed(2)} unbilled`
          : l.accruedUsd != null
            ? `$${l.accruedUsd.toFixed(2)} accrued${l.collectedUsd != null ? ` · $${l.collectedUsd.toFixed(2)} collected` : ""} · $${(l.unbilledUsd ?? 0).toFixed(2)} unbilled`
            : l.estimate
              ? `counted $${countedEstimate(l.estimate).toFixed(2)} as used (1.2 × high) · estimate $${l.estimate.low.toFixed(2)}–$${l.estimate.high.toFixed(2)} ${esc(l.estimate.period)}`
              : "not estimable from local records";
      const stale = l.staleEstimate
        ? ` <span class="badge near">stale estimate — unreconciled &gt;1 invoice cycle</span>`
        : "";
      return `<div><strong>${esc(l.label)}</strong>${stale}: ${figures}<br><span class="muted">${esc(l.note)}</span><br>${freshnessStamp(l.measuredAt, l.sourceClass, 7)}</div>`;
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
  const extractionGap =
    svc.extractionHealth?.status === "gap"
      ? ` <span class="badge blocked" title="${esc(svc.extractionHealth.reason ?? "extractor miss")}">extraction gap</span>`
      : "";
  return `<div class="card${f.stale ? " stale" : ""}"><div><strong><a href="/quota/service/${esc(svc.id)}">${esc(svc.name)}</a></strong>${svc.status !== "active" ? ` <span class="badge src">${esc(svc.status)}</span>` : ""}${conflict}${extractionGap}<br><span class="muted">${esc(svc.plan)}</span><br>${freshnessStamp(svc.measuredAt, svc.sourceClass, svc.cadenceDays)}${svc.nextDate ? `<br><span class="muted">next: ${esc(svc.nextDate)}</span>` : ""}</div><div>${metrics}${billed}${payg}${pace}<div><a href="/quota/service/${esc(svc.id)}">show all →</a></div></div></div>`;
}

function uninstrumentedServicesHtml(services: UsageView["uninstrumentedServices"]): string {
  if (!services.length) return "";
  const rows = services
    .map(
      (svc) =>
        `<tr><th scope="row">${esc(svc.name)}</th><td>${esc(svc.whatFor)}</td><td>${esc(svc.status)}</td><td>${esc(svc.checkedAt ?? "no check date recorded")}</td><td>${esc(svc.overageBehavior)}</td><td>${esc(svc.guardrailNotes)}</td></tr>`,
    )
    .join("");
  return `<h2>Tracker services without instrumented quotas</h2><details><summary>${services.length} evaluated services — expand for tracker states and guardrails</summary><div class="tblwrap"><table><caption>Services in free-services-to-check.md</caption><tr><th scope="col">Service</th><th scope="col">What for</th><th scope="col">Tracker state</th><th scope="col">Checked</th><th scope="col">Overage behavior</th><th scope="col">Guardrail notes</th></tr>${rows}</table></div></details>`;
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
function quotaRefreshHtml(refresh?: import("./quota-refresh.ts").RefreshStatus, generatedAt?: string | null): string {
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
  /** Analytics & projection model (2026-11-13): built by the server
   *  from the route ledger + snapshot store; absent/null renders no
   *  section (older callers and tests are unaffected). */
  analytics?: import("./analytics-store.ts").QuotaAnalytics | null,
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
    ...(view.choices ? [...view.choices.hasHeadroom, ...view.choices.burnBeforeReset, ...view.choices.doNotStart] : []),
    ...(view.credentialLifecycle ?? []).map((c) => c.providerName),
  ]);
  const display = (name: string) => displayNames.get(name) ?? name;
  const credentialLifecycle = view.credentialLifecycle?.length
    ? `<h2>Credential lifecycle</h2><div class="card">${view.credentialLifecycle
        .map((credential) => {
          const expiry = credential.expiresAt ? ` · expires ${fmtCentral(credential.expiresAt)}` : " · expiry unknown";
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
              ? `${esc(a.alert ?? a.status ?? "attention")}${a.alertSince ? ` · since ${fmtCentral(a.alertSince)}` : ""}${a.sessionExpiresAt ? ` · session expires ${fmtCentral(a.sessionExpiresAt)}` : ""}${a.lastGoodAt ? ` · last good ${fmtCentral(a.lastGoodAt)}` : ""}`
              : a.reporting === "stale"
                ? `keeper not reporting · last check ${fmtCentral(a.checkedAt)}`
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
          return `<div>${esc(w.name)}: <meter min="0" max="100" value="${pct.toFixed(0)}"></meter> ${pct.toFixed(0)}% · ${used}/${limit} ${esc(w.unit)} · resets ${fmtCentral(w.resetsAt)}</div>`;
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
        ? `<div class="muted">gatherer · ${esc(p.freshness.state)} · measured ${fmtCentral(p.freshness.measuredAt)} · ${esc(p.freshness.source)}${p.freshness.error ? ` · ${esc(p.freshness.error)}` : ""}</div>`
        : "";
      const extractionGap =
        p.extractionHealth?.status === "gap"
          ? ` <span class="badge blocked" title="${esc(p.extractionHealth.reason ?? "extractor miss")}">extraction gap</span>`
          : "";

      return `<div class="card pool"><div><strong>${esc(display(p.provider))}</strong><br><span class="muted">${esc(p.plan)}</span><br><span class="badge ${esc(p.state)}">${esc(p.state)}${p.wallLiftsAt ? ` · lifts ${fmtCentral(p.wallLiftsAt)}` : ""}</span>${extractionGap}${p.burnTokensPerMin ? ` <span class="muted">${p.burnTokensPerMin} tok/min</span>` : ""}</div><div>${windows}${credRows}${sessions}${fresh}${p.note ? `<div class="muted">${esc(p.note)}</div>` : ""}</div></div>`;
    })
    .join("");
  const burn = view.burn
    .map(
      (b) =>
        `<tr><td>${esc(display(b.provider))}</td><td>${esc(display(b.pool))}</td><td>${fmtCentral(b.from)}</td><td>${fmtCentral(b.to)}</td><td>${b.tokens.toLocaleString()}</td></tr>`,
    )
    .join("");
  const choices = (list: string[]) => (list.length ? list.map((id) => esc(display(id))).join(", ") : "—");
  const burnAndChoices = `<h2>Burn between resets</h2><div class="tblwrap"><table class="table table-zebra"><caption>Burn between resets</caption><tr><th scope="col">Provider</th><th scope="col">Pool</th><th scope="col">From</th><th scope="col">To</th><th scope="col">Tokens</th></tr>${burn || '<tr><td colspan="5" class="muted">none</td></tr>'}</table></div>
<h2 id="model-choice">Model choice</h2><div class="tblwrap"><table class="table table-zebra"><caption>Model choice rules</caption><tr><th scope="col">Rule</th><th scope="col">Pools</th></tr>
<tr><td>Headroom for a new session</td><td>${choices(view.choices.hasHeadroom)}</td></tr>
<tr><td>Burn-it-before-reset (unused headroom)</td><td>${choices(view.choices.burnBeforeReset)}</td></tr>
<tr><td>Do not start a second session</td><td>${choices(view.choices.doNotStart)}</td></tr></table></div>`;
  if (!usage) {
    return layout(
      "Quota + throughput",
      `<h1>Quota + throughput</h1><p class="muted">All pooled quotas, resets, and burn rates — all times Central (America/Chicago). Machine-first: the same data is at <a href="/api/quota.json">/api/quota.json</a>.</p>
${credentialLifecycle}
${providerAuthSection}
${quotaRefreshHtml(refresh, view.gatherer?.generatedAt ?? null)}
${view.gatherer ? `<p class="muted">Gatherer tick: ${view.gatherer.tickAt ? fmtCentral(view.gatherer.tickAt) : "unknown"} · ${view.gatherer.ok ? "ok" : "failed"}${view.gatherer.tickAt && Date.now() - Date.parse(view.gatherer.tickAt) > 20 * 60_000 ? " · stale" : ""}${view.gatherer.durationMs != null ? ` · ${esc(view.gatherer.durationMs)} ms` : ""}${view.gatherer.error ? ` · ${esc(view.gatherer.error)}` : ""}</p>` : ""}
<h2>Pools</h2>${pools}
${analytics ? analyticsSectionHtml(analytics, display) : ""}
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
    `<h1>Usage &amp; billing</h1><p class="muted">Token quotas, PAYG dollars, and SaaS usage vs limits — from dated local records; every card carries its measured-at stamp, and stale cards dim — all times Central (America/Chicago). Machine-first: <a href="/api/quota.json">/api/quota.json</a>.</p>
${attentionStripHtml(usage)}
${credentialLifecycle}
${providerAuthSection}
${quotaRefreshHtml(refresh, view.gatherer?.generatedAt ?? null)}
${view.gatherer ? `<p class="muted">Gatherer tick: ${view.gatherer.tickAt ? fmtCentral(view.gatherer.tickAt) : "unknown"} · ${view.gatherer.ok ? "ok" : "failed"}${view.gatherer.tickAt && Date.now() - Date.parse(view.gatherer.tickAt) > 20 * 60_000 ? " · stale" : ""}${view.gatherer.durationMs != null ? ` · ${esc(view.gatherer.durationMs)} ms` : ""}${view.gatherer.error ? ` · ${esc(view.gatherer.error)}` : ""}</p>` : ""}
<h2>Token quotas</h2>${pools}
${analytics ? analyticsSectionHtml(analytics, display) : ""}
${paygCardHtml(usage)}
<h2>SaaS &amp; services</h2>${groups || '<p class="muted">No services extracted yet.</p>'}${uninstrumentedServicesHtml(usage.uninstrumentedServices ?? [])}
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

export function sessionCard(
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
export function tierItemCard(
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
  const titleHtml =
    item.ref && isValidRef(item.ref) ? `<a href="${esc(refUrl(item.ref))}">${esc(item.title)}</a>` : esc(item.title);
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
