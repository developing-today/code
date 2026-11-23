// render.ts split (#13051): sessions, conflicts, pages, health, discuss and comments-queue views.
// render.ts re-exports every public name from here; callers keep importing "./render.ts".

import { diffCoverage, staleIssueState, type CoverageSnapshotFile } from "./coverage.ts";
import { registryQueryString, type RegistryView } from "./registry-view.ts";
import type { DashAction } from "./actions.ts";
import { commentKindCounts, queuedComments } from "./comments.ts";
import { ageMsText } from "./feed.ts";
import type { HealthView, PageHealth } from "./health.ts";
import type { DashComment, Provenance, ProvenanceDetail, SessionRecord } from "./model.ts";
import { PAGES } from "./pages.ts";
import { CANONICAL_STATUSES, STATUS_SOURCE_LABELS, STATUS_TAXONOMY } from "./status-taxonomy.ts";
import type { ConflictEntry, ConflictInbox } from "./conflicts.ts";
import type { AuthorIdentity } from "./registry.ts";

import {
  esc,
  fmtDate,
  COMMENT_SCRIPT,
  ACTION_SCRIPT,
  freshnessChips,
  layout,
  sessionObjectRef,
  dashCommentHtml,
  type CommentFormOpts,
  commentThreadHtml,
} from "./render-core.ts";
import {
  FIXTURE_COMMENTS_CAUSE,
  commentsFixtureSourced,
  fixtureCommentsBannerHtml,
  sessionRecordLine,
} from "./render-board.ts";

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
          return `<tr><td><code>${esc(s.key)}</code></td><td>${status}</td><td>${s.ageMs !== undefined ? esc(ageMsText(s.ageMs)) : "—"}</td><td>${s.behindBy === null ? "unknown" : s.behindBy}</td><td>${s.lastSyncAt ? esc(s.lastSyncAt) : "—"}</td><td>${s.lagNote ? esc(s.lagNote) : ""}</td><td>${s.note ? esc(s.note) : ""}</td></tr>`;
        })
        .join("")
    : `<tr><td colspan="7" class="muted">No readable fleet feed — sections cannot be reported.</td></tr>`;
  const gathererRows = view.quotaGatherer?.providers.length
    ? view.quotaGatherer.providers
        .map(
          (p) =>
            `<tr><td><code>${esc(p.providerId)}</code>${p.credentialLabel ? ` · ${esc(p.credentialLabel)}` : ""}</td><td>${esc(p.serviceId)}</td><td>${esc(p.state)}</td><td>unknown</td><td>${p.lastSyncAt ? esc(p.lastSyncAt) : "—"}</td><td>${esc(p.lagNote)}</td></tr>`,
        )
        .join("")
    : `<tr><td colspan="6" class="muted">No quota-gatherer provider rows are available.</td></tr>`;
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
  // #3976: push health — the freshness journal + reconciler state,
  // rendered beside (never merged into) the per-source freshness
  // story: the push state describes the delivery machinery, each
  // source row describes that source's own data age and lag.
  const push = view.push;
  const pushSection = push
    ? `<h2 id="push-health">Push health \u2014 event-driven freshness (#3976)</h2>
<div class="chips fresh" aria-label="Push freshness"><span class="chip">push: ${esc(push.state)}</span></div>
<div class="card"><span class="dot ${push.state === "up" ? "up" : push.state === "down" ? "down" : "stale"}"></span><strong>${esc(push.state)}</strong> \u2014 ${esc(push.detail)}<br>last event ${push.lastEventAt ? esc(ageMsText(Math.max(0, nowMs - Date.parse(push.lastEventAt)))) : "none recorded"} \u00b7 last successful reconcile ${push.lastSuccessfulReconcileAt ? esc(ageMsText(Math.max(0, nowMs - Date.parse(push.lastSuccessfulReconcileAt)))) : "never in this process"} \u00b7 journal cursor <code>${push.cursor ?? "unknown"}</code> \u00b7 gaps: <strong>${push.gaps.open} open</strong> \u00b7 ${push.gaps.repairedTotal} repaired by reconciliation \u00b7 ${push.gaps.resetsTotal} journal reset(s)${push.gaps.lastResetAt ? ` (last ${esc(push.gaps.lastResetAt)})` : ""}</div>
<div class="tblwrap"><table class="table table-zebra"><caption>Push sources \u2014 freshness is per source; a quiet source is silent, not an outage</caption><tr><th scope="col">Source</th><th scope="col">State</th><th scope="col">Last event</th><th scope="col">Age</th><th scope="col">Lag</th><th scope="col">Notices</th></tr>${push.sources.map((s) => `<tr><td><code>${esc(s.source)}</code></td><td>${esc(s.state)}${s.missingNotice ? ' \u00b7 <span class="badge stale">notice missing \u2014 gap open</span>' : ""}</td><td>${s.lastEventAt ? esc(s.lastEventAt) : "\u2014"}</td><td>${s.ageMs !== null ? esc(ageMsText(s.ageMs)) : "\u2014"}</td><td>${s.lagMs !== null ? esc(ageMsText(s.lagMs)) : "\u2014"}</td><td>${s.notices}</td></tr>`).join("")}</table></div>`
    : `<h2 id="push-health">Push health \u2014 event-driven freshness (#3976)</h2>\n<p class="muted">Push health is not assembled in this mode \u2014 no state is invented for it.</p>`;
  const sloRows = (view.slo ?? [])
    .map((s) => {
      const current = !s.sourced
        ? `<span class="muted">unknown / no signal source</span>`
        : s.currentValue !== null
          ? esc(ageMsText(s.currentValue))
          : s.latestBad === null
            ? `<span class="muted">\u2014</span>`
            : s.latestBad
              ? "down"
              : "up";
      const remaining = s.budgetRemainingPercent === null ? "\u2014" : `${s.budgetRemainingPercent.toFixed(1)}%`;
      const burn = s.burnRate1h === null ? "\u2014" : s.burnRate1h.toFixed(2);
      return `<tr><td>${esc(s.label)}</td><td>${esc(s.target)}</td><td>${current}</td><td>${remaining}</td><td>${burn}</td><td>${esc(s.state)}</td></tr>`;
    })
    .join("");
  const sloSection = view.slo
    ? `<h2 id="slo-budget">SLO error budgets (#9988)</h2>
<p class="muted">Each minute the health view is sampled into an append-only journal. Budgets are computed over each facet's rolling window; a facet with no signal source is shown as unknown, never as good.</p>
<div class="tblwrap"><table class="table table-zebra"><caption>SLO error budgets \u2014 remaining budget and 1-hour burn rate per facet</caption><tr><th scope="col">Facet</th><th scope="col">Target</th><th scope="col">Current</th><th scope="col">Budget remaining</th><th scope="col">1h burn</th><th scope="col">State</th></tr>${sloRows}</table></div>`
    : "";
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
${pushSection}
${sloSection}
<h2>Feed sections</h2>
<div class="tblwrap"><table><caption>Feed sections — presence, age, lag, and last sync</caption><tr><th scope="col">Section</th><th scope="col">Present</th><th scope="col">Age</th><th scope="col">Behind by</th><th scope="col">Last sync</th><th scope="col">Lag note</th><th scope="col">Note</th></tr>${sectionRows}</table></div>
<h2>Quota gatherer sources</h2>
<div class="tblwrap"><table><caption>Quota-gatherer provider freshness and lag</caption><tr><th scope="col">Provider</th><th scope="col">Service</th><th scope="col">State</th><th scope="col">Behind by</th><th scope="col">Last sync</th><th scope="col">Lag note</th></tr>${gathererRows}</table></div>
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
  // #1500: the same fixture banner + honest empty wording as the item
  // page, keyed off the comments provenance this page already receives.
  const discussFixtureBanner = fixtureCommentsBannerHtml(provenance);
  const discussEmptyWording =
    commentsFixtureSourced(provenance) && comments.length === 0
      ? `<p class="muted">no comments loaded — ${FIXTURE_COMMENTS_CAUSE}</p>`
      : "";
  const body = `<h1>${esc(title)}</h1><p class="muted">object: <code>${esc(objectRef)}</code></p>${back}${discussFixtureBanner}${discussEmptyWording}
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
export function coverageSectionHtml(coverage: CoverageSnapshotFile | null | undefined): string {
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
    .map((r) => {
      const state = staleIssueState(r);
      const status =
        state === "error"
          ? `Collection error${r.staleIssuesError ? `: ${esc(r.staleIssuesError)}` : ""}`
          : state === "stale"
            ? "STALE — collection is over 7 days old"
            : state === "unavailable"
              ? "Unavailable — collection failed or not collected"
              : "Current";
      const collectedAt = r.staleIssuesCollectedAt
        ? `<time datetime="${esc(r.staleIssuesCollectedAt)}">${esc(r.staleIssuesCollectedAt)}</time>`
        : "—";
      return `<tr><td>${esc(r.repo)}</td><td>${cell(r.repo, r.openIssues, "openIssues")}</td><td>${cell(r.repo, r.gateNow, "gateNow")}</td><td>${cell(r.repo, r.gateNext, "gateNext")}</td><td>${cell(r.repo, r.poolItems, "poolItems")}</td><td>${cell(r.repo, r.poolUnsetStatus, "poolUnsetStatus")}</td><td>${cell(r.repo, r.poolUnsetArea, "poolUnsetArea")}</td><td>${r.staleOpenIssues === null ? "—" : r.staleOpenIssues}</td><td>${collectedAt}</td><td>${status}</td></tr>`;
    })
    .join("");
  const note = baseline
    ? `Week-over-week delta vs the ${esc(fmtDate(baseline.generatedAt))} snapshot in parentheses. Snapshot stored ${esc(fmtDate(current.generatedAt))}; it rotates into the baseline after a week.`
    : `First snapshot stored ${esc(fmtDate(current.generatedAt))} — the week-over-week delta appears after the next weekly rotation.`;
  return `<h2>Coverage — pool vs backlog (#1220)</h2>
 <div class="tblwrap"><table><caption>Coverage — pool vs backlog by repository</caption><thead><tr><th scope="col">Repo</th><th scope="col">Open issues</th><th scope="col">now gate</th><th scope="col">next gate</th><th scope="col">Pool items</th><th scope="col">Unset Status</th><th scope="col">Unset Area</th><th scope="col">Open issues untouched &gt;90 days</th><th scope="col">Stale count collected at (UTC)</th><th scope="col">Stale count status</th></tr></thead><tbody>${rows}</tbody></table></div>
<p class="muted">${note} Open issues and gate labels come from the collector's Search API census in the fleet feed; pool counts from Project #4 (— = that source was unavailable for the repo, never a real zero). The stale count includes open issues only, is captured at the per-repository timestamp, and is marked stale after seven days; missing/error data is not presented as zero. The same report is in <a href="/api/projects.json">/api/projects.json</a> as \`coverage\`.</p>`;
}

/** The Projects pool body (cards, chips, coverage) without the page
 * shell — rendered standalone by renderProjects and, since the #1497
 * merge, as the pool section of /program. */
/** The per-item brief panel (#1279): visibly labelled as derived, every
 * line carrying its evidence link. The identical brief object ships in
 * the JSON twin (/api/projects.json `briefs`). */
/** The canonical status taxonomy (#1198 acceptance 1): the mapping
 * table from status-taxonomy.ts, rendered where the pool renders —
 * /projects standalone and the pool section of /program. */
export function statusTaxonomyHtml(): string {
  const rows = STATUS_TAXONOMY.map(
    (row) =>
      `<tr><td>${esc(STATUS_SOURCE_LABELS[row.source])}</td><td>${esc(row.raw)}</td><td>${esc(row.canonical)}</td></tr>`,
  ).join("");
  return `<h2>Canonical status taxonomy (#1198)</h2>
<p class="muted">One canonical status set — ${CANONICAL_STATUSES.map(esc).join(" · ")} — and every source's vocabulary mapped into it. An item's canonical status is the same on every page for the same inputs; a source value with no row here is unmapped and surfaces as a ⚠ disagreement rather than being guessed. The ⚠ badges on Board, Roadmap, Today, and this pool all compare through this table.</p>
<div class="tblwrap"><table><caption>Per-source status mapping to the canonical set</caption><thead><tr><th scope="col">Source</th><th scope="col">Source status</th><th scope="col">Canonical status</th></tr></thead><tbody>${rows}</tbody></table></div>`;
}
