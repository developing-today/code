// render.ts split (#13051): projects and program views, epic graph and phase panels.
// render.ts re-exports every public name from here; callers keep importing "./render.ts".

import { renderDesignBoards, type BoardSnapshot } from "./design-boards.ts";
import { timeline as buildTimeline } from "./derive.ts";
import type { CoverageSnapshotFile } from "./coverage.ts";
import type { ForecastContract } from "./forecast-contract.ts";
import type { FlowAnalytics } from "./flow-analytics.ts";
import { flowChartsHtml } from "./flow-charts.ts";
import type { DashAction } from "./actions.ts";
import { cardState } from "./card-state.ts";
import { serviceHomeLinks } from "./service-links.ts";
import {
  type GhItem,
  type OrchestrationSection,
  type ProjectAudit,
  type ProjectPoolItem,
  type Provenance,
  type ProvenanceDetail,
  type RefKey,
  isValidRef,
  refUrl,
} from "./model.ts";
import type { ProgramData, ProgramEntry, ProgramLive, TrackedIssue } from "./program.ts";
import type { ProjectDetail, ProjectSummary } from "./project-config.ts";
import { derivedProjectState, PROJECT_URL } from "./projects.ts";
import {
  PHASE12_ISSUE_URL,
  joinPhase12Steps,
  parsePhase12Steps,
  phase12IsStale,
  type Phase12Snapshot,
  type Phase12StepState,
} from "./phase12.ts";
import {
  PHASE_TRACKER_REPO,
  PHASE_TRACKER_UMBRELLA,
  derivePhase12Progress,
  derivePhaseValidation,
} from "./phase-tracker.ts";
import { deriveMappedProgress, type PhaseMappingSnapshot } from "./phase-mapping.ts";
import {
  BUILD_HEALTH_REPO,
  buildHealthIsStale,
  projectLane,
  projectNightlyTrend,
  type BuildHealthSnapshot,
} from "./build-health.ts";
import type { QueueSnapshot } from "./queue.ts";
import {
  F12_DESIGN_ISSUE_URL,
  parseF12Designs,
  parseF12Items,
  projectF12Readiness,
  type F12PrRef,
} from "./f12-readiness.ts";
import { canonicalStatusFor } from "./status-taxonomy.ts";
import { effectiveProjectDisagreement, resolveTrustedProjectValue } from "./conflicts.ts";
import type { ConflictRuling } from "./conflict-rulings.ts";
import { buildItemBrief, type ItemBrief } from "./brief.ts";
import { buildEpicRollups, uniqueProjectItemsByRef, type EpicRollup } from "./epics.ts";
import { spendCardHtml, unattributedSpendHtml, type SpendCardsView } from "./spend-cards.ts";
import { effortWeight, handsLine, summarizeWeights, weightedLine } from "./weights.ts";
import { blockedSummary, depthLayout, hasGraphEdges } from "./graph.ts";
import { EPIC_GRAPH_DEFAULT_CAP, epicGraph, epicSelector } from "./epic-graph.ts";
import type { AuthorIdentity } from "./registry.ts";

import {
  esc,
  fmtDate,
  ACTION_SCRIPT,
  freshnessChips,
  layout,
  sessionObjectRef,
  ghObjectRef,
  tierItemObjectRef,
  actionBadgesHtml,
  actionHistoryHtml,
  triageActionsHtml,
  needsUserBodyHtml,
  needsUserFormsHtml,
  statusRequestHtml,
  sessionVerbSupport,
  sessionActionsHtml,
  itemBadge,
  briefPanelHtml,
} from "./render-core.ts";
import { forecastSummaryHtml, timelineTable, epicRollupHtml } from "./render-board.ts";
import { renderSectionPrompt } from "./prompt-widget.ts";
import { coverageSectionHtml, statusTaxonomyHtml } from "./render-views.ts";

export function projectsBodyHtml(
  pool: ProjectPoolItem[],
  opts: {
    area?: string | null;
    /** Pool facet filters (#1208): Project Status / Kind single-select values, or the "(no status)" / "(no kind)" unset buckets. */
    status?: string | null;
    kind?: string | null;
    /** Project #4 audit snapshot (#1208): reconciles the rendered pool count against the board total (draft/unlinked caveat). */
    audit?: ProjectAudit | null;
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
    /** Shared forecast contracts by item ref (#3984): the same objects
     * /api/forecasts.json serves, rendered as one-line summaries. */
    forecasts?: Map<RefKey, ForecastContract[]>;
    /** Spend beside progress (#4824): the #4823 rollup store's
     * reader view. Absent/null: cards render exactly as before. */
    spend?: SpendCardsView | null;
    /** Slice-2 trust ruling (#1282): a standing project-vs-derived
     *  ruling suppresses the ⚠ clash badge — the card applies the
     *  trusted value and says so, instead of showing both sides. */
    ruling?: ConflictRuling | null;
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
  const shownByArea =
    area === UNSET_AREA ? pool.filter((p) => !p.area) : area ? pool.filter((p) => p.area === area) : pool;
  // #1208: Status and Kind are pool fields too — facet-filter them like
  // Area (the saved-view dimensions in views.ts carry the same pair, so
  // a view and the chips agree). "(no status)" / "(no kind)" are the
  // explicit unset buckets, mirroring the Area "(unset)" bucket (#1143).
  const UNSET_STATUS = "(no status)";
  const UNSET_KIND = "(no kind)";
  const statuses = [...new Set(pool.map((p) => p.status).filter((s): s is string => Boolean(s)))].sort();
  const kinds = [...new Set(pool.map((p) => p.kind).filter((s): s is string => Boolean(s)))].sort();
  const statusSel =
    opts.status === UNSET_STATUS ? UNSET_STATUS : opts.status && statuses.includes(opts.status) ? opts.status : null;
  const kindSel = opts.kind === UNSET_KIND ? UNSET_KIND : opts.kind && kinds.includes(opts.kind) ? opts.kind : null;
  const shown = shownByArea
    .filter((p) => (statusSel === UNSET_STATUS ? !p.status : statusSel ? p.status === statusSel : true))
    .filter((p) => (kindSel === UNSET_KIND ? !p.kind : kindSel ? p.kind === kindSel : true));
  /** Facet link builder: change one facet, keep the others. */
  const facetHref = (patch: { area?: string | null; status?: string | null; kind?: string | null }): string => {
    const q = new URLSearchParams();
    const a = patch.area !== undefined ? patch.area : area;
    const s = patch.status !== undefined ? patch.status : statusSel;
    const k = patch.kind !== undefined ? patch.kind : kindSel;
    if (a) q.set("area", a);
    if (s) q.set("status", s);
    if (k) q.set("kind", k);
    const qs = q.toString();
    return qs ? `${base}?${qs}` : base;
  };
  const chips = [
    `<a href="${facetHref({ area: null })}"${!area ? ' class="active"' : ""}>all (${pool.length})</a>`,
    ...areas.map(
      (a) =>
        `<a href="${facetHref({ area: a })}"${a === area ? ' class="active"' : ""}>${esc(a)} (${pool.filter((p) => p.area === a).length})</a>`,
    ),
    ...(unsetCount > 0
      ? [
          `<a href="${facetHref({ area: UNSET_AREA })}"${area === UNSET_AREA ? ' class="active"' : ""}>${UNSET_AREA} (${unsetCount})</a>`,
        ]
      : []),
  ].join("");
  if (!pool.length) {
    return `<div class="banner">No Project pool in the current fleet feed — the collector embeds it from the org Project <a href="${PROJECT_URL}">Developing Today (#4)</a> when a Projects token is available on the sandbox. Items on the board/roadmap fall back to the projector/fixtures chain until the pool lands.</div>${coverageSectionHtml(opts.coverage)}${opts.spend?.unattributed ? unattributedSpendHtml(opts.spend.unattributed) : ""}<p class="muted">Live services: ${serviceHomeLinks()
      .map((s) => `<a href="${esc(s.url)}">${esc(s.label)} ↗</a>`)
      .join(" · ")}</p>`;
  }
  const areaWeights = areas
    .map((a) => {
      const items = pool.filter((p) => p.area === a);
      const w = summarizeWeights(uniqueProjectItemsByRef(items));
      return `<div class="card"><strong>${esc(a)}</strong> — ${esc(weightedLine(w))}<br><span class="muted">${esc(handsLine(w))}</span></div>`;
    })
    .join("");
  const shownWeights = summarizeWeights(uniqueProjectItemsByRef(shown));
  const epicRollups = buildEpicRollups(pool);
  const cards = shown
    .map((p) => {
      const detected = derivedProjectState(p, opts.orchestration);
      // #1282 slice 2: a standing ruling suppresses the clash — the
      // card applies the trusted value and labels it, so the page
      // cannot show a ⚠ the inbox no longer lists.
      const clash = effectiveProjectDisagreement(p, opts.orchestration, undefined, opts.ruling ?? null);
      const trusted = !clash && opts.ruling?.classId === "project-vs-derived" && p.status ? opts.ruling.trusted : null;
      const trustedValue = resolveTrustedProjectValue(p, opts.orchestration, undefined, opts.ruling ?? null);
      const appliedStatus = trustedValue ?? p.status;
      const appliedDerived = trustedValue ?? detected;
      const clashBadge = clash
        ? ` <span class="badge stale" title="The Project Status and the state the rest of the dashboard derives disagree — both are shown, neither is preferred">⚠ project: ${esc(clash.project)} · derived: ${esc(clash.derived)}</span>`
        : trusted
          ? ` <span class="badge src" title="A trust ruling stands for Project Status vs derived state (recorded in the source-conflict inbox): this card applies the trusted ${esc(trusted)} value while the disagreement is suppressed — revoke the ruling to re-surface it">trusts ${esc(trusted)}</span>`
          : "";
      const statusBadge = appliedStatus
        ? `<span class="badge ${appliedStatus === "Done" ? "merged" : appliedStatus === "Todo" ? "closed" : "implementing"}">${esc(appliedStatus)}</span>`
        : `<span class="badge">no status</span>`;
      const linksLine = `<span class="muted"><a href="${esc(p.issueUrl)}">${esc(p.repo)}#${p.number}</a> · <a href="${esc(p.url)}">Project item</a> · <a href="${esc(p.projectUrl)}">Project #4</a>${p.updatedAt ? ` · updated ${fmtDate(p.updatedAt)}` : ""}</span>`;
      // #5082: only active work (canonical Project status
      // In Progress / In Review) and epics
      // render as a full card — status-request control, derived
      // brief, rollups. An epic's rollup / forecast / spend ARE its
      // substance at any status (#3984, #4824) and epics are few
      // (40 of 1,422). Every other item renders compact: title,
      // badges, links — the shape the per-project pages already
      // use. The pool only ever
      // grows — 725 of 1,422 items were Done at the 2026-11-13
      // measurement — and at ~2.8 KB per full card (actbox ~1.2 KB +
      // brief ~0.9 KB of it) the page reached 4.08 MB and grew with
      // every filed item. A compact row keeps the item's identity,
      // status, badges and links; the dropped blocks are the ones
      // whose content is formulaic for a queued/finished item (the
      // brief's "Next: No action" for Done items) or an action that
      // belongs to active work. When the derived state disagrees the
      // ⚠ clash badge still shows on the compact row, so no signal
      // is lost. Counts, chips and weights use the whole pool either
      // way; the area filter and saved views filter rows the same.
      // Action-result badges (a pending / applied status request)
      // also stay on compact rows — only the form and brief drop.
      const canonical = canonicalStatusFor("project", p.status);
      const active = canonical === "In Progress" || canonical === "In Review" || p.kind === "Epic";
      const objectRef = ghObjectRef(p.ref);
      const actionBadges = actionBadgesHtml(opts.actionsByObject?.get(objectRef));
      if (!active) {
        const metaCompact = [
          p.area ? `<span class="badge src">${esc(p.area)}</span>` : "",
          p.horizon ? `<span class="badge src">${esc(p.horizon)}</span>` : "",
          p.kind ? `<span class="badge src">${esc(p.kind)}</span>` : "",
          `<span class="badge ${p.state === "open" ? "open" : "closed"}">issue ${esc(p.state)}</span>`,
        ].join(" ");
        const compactLinks = `<span class="muted"><a href="${esc(p.issueUrl)}">${esc(p.repo)}#${p.number}</a> · <a href="${esc(p.url)}">Project item</a>${p.updatedAt ? ` · updated ${fmtDate(p.updatedAt)}` : ""}</span>`;
        return `<div class="card"><strong>${esc(p.title)}</strong>${clashBadge}<br>${statusBadge} ${metaCompact}<br>${compactLinks}${actionBadges}</div>`;
      }
      const meta = [
        p.area ? `<span class="badge src">${esc(p.area)}</span>` : "",
        p.horizon ? `<span class="badge src">${esc(p.horizon)}</span>` : "",
        p.kind ? `<span class="badge src">${esc(p.kind)}</span>` : "",
        `<span class="badge ${p.state === "open" ? "open" : "closed"}">issue ${esc(p.state)}</span>`,
        `<span class="badge src">derived: ${esc(appliedDerived)}</span>`,
      ].join(" ");
      // #1154: the status-request control + the store's visible state.
      const statusCtl = opts.identities
        ? `${actionBadges}${statusRequestHtml(objectRef, p.ref, p.status ?? null, { identities: opts.identities })}`
        : "";
      const rollup = p.kind === "Epic" ? epicRollups.get(p.ref) : undefined;
      const epicRollup = rollup ? epicRollupHtml(rollup) : "";
      const spendCard = opts.spend?.byScope.get(p.ref);
      const spendHtml = spendCard ? spendCardHtml(spendCard) : "";
      const brief =
        opts.briefs?.get(p.ref) ??
        buildItemBrief(p, { pool, orchestration: opts.orchestration, ruling: opts.ruling ?? null });
      const forecastHtml = forecastSummaryHtml(opts.forecasts?.get(p.ref));
      return `<div class="card"><strong>${esc(p.title)}</strong>${clashBadge}<br>${statusBadge} ${meta}<br>${linksLine}${epicRollup}${spendHtml}${forecastHtml}${statusCtl}${briefPanelHtml(brief)}</div>`;
    })
    .join("");
  const body = `<p class="muted">${pool.length} items from the org Project <a href="${PROJECT_URL}">Developing Today (#4)</a>, read live by the fleet-feed collector (GraphQL, read-only) and carried in the feed. This pool is the resolution source for board/roadmap GitHub items. Where the Project Status and the derived state disagree, the card shows ⚠ with both values — unless a trust ruling stands for the class, in which case the card applies the trusted value and says so. Machine-first: <a href="/api/projects.json">/api/projects.json</a>.</p>
<div class="chips">${chips}</div>
<div class="chips"><span class="muted">Status:</span> ${[
    `<a href="${facetHref({ status: null })}"${!statusSel ? ' class="active"' : ""}>any</a>`,
    ...statuses.map(
      (s) =>
        `<a href="${facetHref({ status: s })}"${s === statusSel ? ' class="active"' : ""}>${esc(s)} (${pool.filter((p) => p.status === s).length})</a>`,
    ),
    ...(pool.some((p) => !p.status)
      ? [
          `<a href="${facetHref({ status: UNSET_STATUS })}"${statusSel === UNSET_STATUS ? ' class="active"' : ""}>${UNSET_STATUS} (${pool.filter((p) => !p.status).length})</a>`,
        ]
      : []),
  ].join("")}</div>
<div class="chips"><span class="muted">Kind:</span> ${[
    `<a href="${facetHref({ kind: null })}"${!kindSel ? ' class="active"' : ""}>any</a>`,
    ...kinds.map(
      (k) =>
        `<a href="${facetHref({ kind: k })}"${k === kindSel ? ' class="active"' : ""}>${esc(k)} (${pool.filter((p) => p.kind === k).length})</a>`,
    ),
    ...(pool.some((p) => !p.kind)
      ? [
          `<a href="${facetHref({ kind: UNSET_KIND })}"${kindSel === UNSET_KIND ? ' class="active"' : ""}>${UNSET_KIND} (${pool.filter((p) => !p.kind).length})</a>`,
        ]
      : []),
  ].join("")}</div>
${opts.audit && (opts.audit.poolTotalItems ?? opts.audit.totalItems) > pool.length ? `<div class="banner">Board total ${opts.audit.poolTotalItems ?? opts.audit.totalItems} items; this pool renders ${pool.length} linked issue/PR items. ${(opts.audit.poolTotalItems ?? opts.audit.totalItems) - pool.length} board item${(opts.audit.poolTotalItems ?? opts.audit.totalItems) - pool.length === 1 ? " is" : "s are"} draft or unlinked (no issue/PR to link to) and not rendered as cards \u2014 counted in the audit in <a href="/api/projects.json">/api/projects.json</a>, never silently dropped.</div>` : ""}
${shown.length !== pool.length ? `<p class="muted">Filters show ${shown.length} of ${pool.length} pool items.</p>` : ""}
${unsetCount > 0 ? `<div class="banner">${unsetCount} pool item${unsetCount === 1 ? "" : "s"} carry no Area — they belong to no project page until Area is set on the Project. Setting Area is the orchestrator's reconciliation job; this count keeps the gap visible. <a href="${base}?area=${encodeURIComponent(UNSET_AREA)}">View the (unset) bucket →</a></div>` : ""}
${coverageSectionHtml(opts.coverage)}
${statusTaxonomyHtml()}
<p class="muted">Weighted progress (effort xs=1 · s=4 · m=12 · l=40, #1108): <strong>${esc(weightedLine(shownWeights))}</strong><br>${esc(handsLine(shownWeights))}</p>
${area ? "" : `<h2>Per-Area weighted progress</h2>${areaWeights}`}
<p class="muted">Live services: ${serviceHomeLinks()
    .map((s) => `<a href="${esc(s.url)}">${esc(s.label)} ↗</a>`)
    .join(" · ")}</p>
${cards || '<p class="muted">No items in this area.</p>'}${opts.spend?.unattributed ? unattributedSpendHtml(opts.spend.unattributed) : ""}`;
  return body;
}

export function renderProjects(
  pool: ProjectPoolItem[],
  provenance: Provenance,
  opts: {
    area?: string | null;
    /** Pool facet filters (#1208): Project Status / Kind single-select values, or the "(no status)" / "(no kind)" unset buckets. */
    status?: string | null;
    kind?: string | null;
    /** Project #4 audit snapshot (#1208): reconciles the rendered pool count against the board total (draft/unlinked caveat). */
    audit?: ProjectAudit | null;
    designBoards?: BoardSnapshot[];
    orchestration?: OrchestrationSection | null;
    detail?: ProvenanceDetail;
    actionsByObject?: Map<string, DashAction[]>;
    identities?: AuthorIdentity[];
    coverage?: CoverageSnapshotFile | null;
    basePath?: string;
    briefs?: Map<RefKey, ItemBrief>;
    forecasts?: Map<RefKey, ForecastContract[]>;
    spend?: SpendCardsView | null;
    /** Slice-2 trust ruling (#1282), passed through to the pool cards. */
    ruling?: ConflictRuling | null;
  } = {},
): string {
  return layout(
    "Projects",
    `<h1>Projects — GitHub Project pool</h1>
${renderDesignBoards(opts.designBoards ?? [])}
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
  return `<div class="card"><strong><a href="${esc(s.route)}">${esc(s.title)}</a></strong><br>${poolLine}<br>${roadmapLine}${homeLine}<br><a href="${esc(s.route)}">Project page →</a>${renderSectionPrompt("program", s.id)}</div>`;
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
    status?: string | null;
    kind?: string | null;
    audit?: ProjectAudit | null;
    designBoards?: BoardSnapshot[];
    coverage?: CoverageSnapshotFile | null;
    orchestration?: OrchestrationSection | null;
    actionsByObject?: Map<string, DashAction[]>;
    identities?: AuthorIdentity[];
    /** Prebuilt briefs (#1279), e.g. with comment activity folded in. */
    briefs?: Map<RefKey, ItemBrief>;
    forecasts?: Map<RefKey, ForecastContract[]>;
    /** The #4829 flow twin the #4830 charts render on this page. */
    flowAnalytics?: FlowAnalytics;
    /** Spend beside progress (#4824) for the pool section. */
    spend?: SpendCardsView | null;
    /** Slice-2 trust ruling (#1282), passed through to the pool cards. */
    ruling?: ConflictRuling | null;
  } | null,
): string {
  const poolSectionHtml = poolSection
    ? `<h2 id="projects-pool">Projects pool</h2>
<p class="muted">The Projects page, merged into Program status by the #1497 curation — the raw org Project pool with its Project-vs-derived disagreements. <code>/projects</code> redirects to this section; the machine twin <a href="/api/projects.json">/api/projects.json</a> is unchanged.</p>
${projectsBodyHtml(poolSection.pool, { area: poolSection.area, status: poolSection.status, kind: poolSection.kind, audit: poolSection.audit, orchestration: poolSection.orchestration, actionsByObject: poolSection.actionsByObject, identities: poolSection.identities, coverage: poolSection.coverage, basePath: "/program", briefs: poolSection.briefs, forecasts: poolSection.forecasts, spend: poolSection.spend, ruling: poolSection.ruling ?? null })}`
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
${renderDesignBoards(poolSection?.designBoards ?? [])}
<h2>Live status</h2>
<p class="muted">Derived at request time from the Project pool and the latest orchestration section — not written by hand.</p>
${poolSummaryHtml(live)}
${orchestrationHtml(live)}
<h2>The PM dashboard's own record</h2>
<p class="muted">This dashboard's architecture, what's built, what's next, and its long-term roadmap live on its project page: <a href="/program/pm-dashboard">PM dashboard →</a>. Its tracked issues, resolved live:</p>
${trackedTable(live.tracked)}${poolSectionHtml}${poolSection?.flowAnalytics ? flowChartsHtml(poolSection.flowAnalytics) : ""}`;
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

interface EpicGraphActionContext {
  orchestration?: OrchestrationSection | null;
  actionsByObject?: Map<string, DashAction[]>;
  identities?: AuthorIdentity[];
  orchestrationState?: string;
}

function epicGraphSectionHtml(
  items: ProjectPoolItem[],
  selectorItems: ProjectPoolItem[] = items,
  actionContext?: EpicGraphActionContext,
): string {
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
  const sections = selector.map((entry) => epicGraphOneHtml(items, entry, actionContext)).join("");
  return `<h3>Per-epic dependency graph</h3>${selectorHtml}${sections}`;
}

function graphNodeActionsHtml(nodeRef: RefKey, context?: EpicGraphActionContext): string {
  const section = context?.orchestration;
  const drewItem = section?.tiers.needsDrew.find((item) => item.ref === nodeRef);
  const orchestratorItem = section?.tiers.needsOrchestrator.find((item) => item.ref === nodeRef);
  const item = drewItem ?? orchestratorItem;
  if (!item) {
    return `<p class="muted epic-graph-actions">No inline action: this item is not in the currently published Needs User or Needs Orchestrator tier.</p>`;
  }

  const tier = drewItem ? "needs-drew" : "needs-orchestrator";
  const objectRef = tierItemObjectRef(item);
  const actions = context?.actionsByObject?.get(objectRef);
  const state = cardState(actions);
  const disabledReason =
    item.source === "watchdog" || context?.orchestrationState === "live"
      ? undefined
      : `the tier source is ${context?.orchestrationState ?? "unavailable"}, so this item cannot be confirmed against the orchestrator's current published queues`;
  const controls = context?.identities
    ? triageActionsHtml(
        objectRef,
        tier,
        { identities: context.identities, ...(disabledReason ? { disabledReason } : {}) },
        tier === "needs-drew" ? needsUserFormsHtml(item, objectRef, Date.now(), state) : "",
        state,
      )
    : `<p class="muted">Inline actions unavailable: no dashboard author identity is configured.</p>`;
  const needsUserBody = tier === "needs-drew" ? needsUserBodyHtml(item, actions) : "";
  const history = `${actionBadgesHtml(actions)}<details class="carddetail"><summary>Action history</summary>${actionHistoryHtml(actions)}</details>`;

  let sessionControls = "";
  if (item.sessionId) {
    const sessionSource = item.source === "claude" ? "claude-code" : item.source;
    const support = sessionVerbSupport(sessionSource);
    if (!context?.identities) {
      sessionControls = `<p class="muted">Session action unavailable: no dashboard author identity is configured.</p>`;
    } else if (support.some((entry) => !entry.reason)) {
      const sessionRef = sessionObjectRef(item.sessionId);
      const sessionActions = context?.actionsByObject?.get(sessionRef);
      sessionControls = `${actionBadgesHtml(sessionActions)}${sessionActionsHtml(item.sessionId, sessionSource, { identities: context.identities })}<details class="carddetail"><summary>Session action history</summary>${actionHistoryHtml(sessionActions)}</details>`;
    } else {
      const reason =
        support.find((entry) => entry.reason)?.reason ??
        "this published item has no supported live session control channel";
      sessionControls = `<p class="muted">Session action unavailable: ${esc(reason)}.</p>`;
    }
  }

  return `<div class="epic-graph-actions"><span class="badge ${tier === "needs-drew" ? "drew" : "orch"}">${tier === "needs-drew" ? "Needs User" : "Needs Orchestrator"}</span>${needsUserBody}${history}${controls}${sessionControls}</div>`;
}

function epicGraphOneHtml(
  items: ProjectPoolItem[],
  entry: ReturnType<typeof epicSelector>[number],
  actionContext?: EpicGraphActionContext,
): string {
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
  const cycleGroupsHtml = g.cycleGroups.length
    ? `<div class="epic-graph-cycle-groups">${g.cycleGroups
        .map((group) => {
          const members = group.members
            .map((ref) => {
              const member = poolByRef.get(ref);
              return `<li>${poolRefLink(ref)}${member?.title ? ` — ${esc(member.title)}` : ""}</li>`;
            })
            .join("");
          const explanation =
            group.kind === "dependency"
              ? "The dependency data is contradictory. Resolving one member alone does not clear this cycle; correct the cycle edges."
              : "The parent links are cyclic. These members are one hierarchy warning group, not independent roots; correct the parent edges.";
          return `<section class="banner" role="alert"><strong>⚠ ${esc(group.name)}</strong><p>${explanation}</p><ul>${members}</ul></section>`;
        })
        .join("")}</div>`
    : "";
  const edgeDiagnosticsHtml = g.edgeDiagnostics.length
    ? `<section class="banner" role="status"><strong>⚠ Relation edge data needs review</strong><ul>${g.edgeDiagnostics
        .map((diagnostic) => {
          const relation =
            diagnostic.relation === "parent" ? "parent" : diagnostic.relation === "blockedBy" ? "blocked-by" : "blocks";
          const issue =
            diagnostic.target === null
              ? "endpoint unavailable"
              : `malformed endpoint <code>${esc(diagnostic.target)}</code>`;
          const state = diagnostic.state === "unavailable" ? "Unavailable edge data" : "Malformed relation reference";
          return `<li>${state}: ${poolRefLink(diagnostic.sourceRef)} ${relation} edge — ${issue}; it was not treated as an external issue.</li>`;
        })
        .join("")}</ul></section>`
    : "";
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
        .map((r) => {
          const blockerItems = r.blockers.map((b) => poolRefLink(b));
          const contributingItems =
            r.contributingItems.length > 0
              ? `<br><span class="muted">Contributing downstream epics: ${r.contributingItems.map((ci) => poolRefLink(ci)).join(", ")}</span>`
              : "";
          return `<li><a href="${esc(r.issueUrl)}">${esc(r.ref)}</a> — waiting on ${r.downstreamCount} distinct downstream open epic item${r.downstreamCount === 1 ? "" : "s"} via ${r.blockers.length} unresolved blocker${r.blockers.length === 1 ? "" : "s"}: ${blockerItems.join(", ")}${contributingItems}</li>`;
        })
        .join("")}</ul>`
    : g.blockedCount > 0
      ? `<p class="muted">${g.blockedCount} open item${g.blockedCount === 1 ? " is" : "s are"} waiting on an unresolved blocker, but none can be ranked here — members of a dependency cycle are excluded from the unblock ranking until the cycle is broken.</p>`
      : `<p class="muted">No open item in this epic is waiting on an unresolved blocker.</p>`;

  const rankedBlockersHtml = g.rankedBlockers.length
    ? `<ol class="epic-graph-list">${g.rankedBlockers
        .map((blocker) => {
          const stateText =
            blocker.state === null ? "outside this pool; status unknown — unresolved" : "open unresolved blocker";
          const downstream = blocker.downstreamItems
            .map((item) => {
              const weightText = item.effortWeight === null ? "unsized" : `effort weight ${item.effortWeight}`;
              const additionalBlockers = item.additionalBlockers.length
                ? `additional unresolved blockers: ${item.additionalBlockers.map((ref) => `<a href="${esc(refUrl(ref))}">${esc(ref)}</a>`).join(", ")}`
                : "no additional unresolved blockers";
              return `<li><a href="${esc(item.issueUrl)}">${esc(item.ref)}</a> (${weightText}) — ${additionalBlockers}</li>`;
            })
            .join("");
          return `<li><a href="${esc(blocker.issueUrl)}">${esc(blocker.ref)}</a> (${stateText}) — ${blocker.downstreamCount} distinct open downstream item${blocker.downstreamCount === 1 ? "" : "s"}; known effort weight ${blocker.knownEffortWeight}; ${blocker.unsizedCount} unsized contributor${blocker.unsizedCount === 1 ? "" : "s"}: <ul>${downstream}</ul></li>`;
        })
        .join("")}</ol>`
    : `<p class="muted">No unresolved blocker has open downstream work in this epic.</p>`;

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
      const actions = isValidRef(n.ref) ? graphNodeActionsHtml(n.ref, actionContext) : "";
      return `<li>${nodeRef} ${name} — ${stateText}; ${blockedBy}; ${blocks}${actions}</li>`;
    })
    .join("")}</ul>`;

  return `<section>${heading}${summary}${cycleGroupsHtml}${edgeDiagnosticsHtml}${truncatedHtml}<div class="epic-graph-flex">${svg}<div class="epic-graph-side"><h5>Primary ranking — blockers by downstream open work</h5><p class="muted">Each open epic item is counted once per blocker across its transitive blockedBy ancestry. This measures downstream reach, not a promise that every item is immediately unblocked. Current additional unresolved blockers are listed per item; unsized contributors count separately from known effort weight. The longest dependency chain remains a separate diagnostic.</p>${rankedBlockersHtml}<h5>Open items by their worst blocker's downstream reach</h5>${rankedHtml}<h5>All items — dependency list</h5>${listHtml}</div></div></section>`;
}

/** The Path to Phase 12 panel (#9922): #1235's ordered steps, parsed
 * from its body snapshot and joined to live GitHub item state and to
 * queue state from the fleet-queue collector (#9920). A stale or
 * missing body snapshot renders the warning INSTEAD of the steps — a
 * silently old step list is the defect #1239's rules name. A stale
 * queue snapshot still renders, but behind its own warning: the step
 * list is the panel's spine, the task states are the perishable part. */
export function phase12PanelHtml(
  snapshot: Phase12Snapshot | null,
  items: Map<RefKey, GhItem>,
  queue: QueueSnapshot | null = null,
): string {
  const head = `<h2>Path to Phase 12 — live from <a href="${PHASE12_ISSUE_URL}">#1235</a></h2>`;
  if (!snapshot) {
    return `${head}<div class="banner">The #1235 body feed is unavailable in the current source, so no step state is shown — a hand-maintained copy would drift, and this panel never renders one.</div>`;
  }
  const queueSegment =
    queue === null
      ? "queue state unavailable in the current source"
      : !queue.available
        ? "queue read failed — per-step task state withheld"
        : `queue state from the fleet-queue collector, fetched ${esc(new Date(queue.fetchedAt).toISOString())}`;
  const sourceLine = `<p class="muted">Source: <a href="${PHASE12_ISSUE_URL}">developing-today/code#1235</a> body, fetched ${esc(snapshot.fetchedAt)} · issue updated ${esc(snapshot.issueUpdatedAt)} · per-step state is live GitHub state; ${queueSegment}.</p>`;
  if (phase12IsStale(snapshot)) {
    return `${head}${sourceLine}<div class="banner">Stale source: this snapshot was fetched at ${esc(snapshot.fetchedAt)}, outside the freshness window — the step list is withheld rather than shown silently old. It refreshes on the next successful body read.</div>`;
  }
  const steps = joinPhase12Steps(parsePhase12Steps(snapshot.body), items, queue);
  if (!steps.length) {
    return `${head}${sourceLine}<div class="banner">No ordered steps could be parsed from the current #1235 body, so no step state is shown.</div>`;
  }
  const queueStale =
    queue !== null && queue.available && queue.stale
      ? `<div class="banner">Stale queue source: task states were fetched at ${esc(new Date(queue.fetchedAt).toISOString())}, outside the freshness window — they are shown behind this warning rather than silently old, and refresh on the next successful broker read.</div>`
      : "";
  const queueBadge = (s: Phase12StepState): string => {
    if (queue === null || !queue.available) return "";
    const [task, ...earlier] = s.queueTasks;
    if (!task) return ` <span class="badge">no queue task</span>`;
    const cls =
      task.status === "claimed"
        ? "implementing"
        : task.status === "done" || task.status === "completed"
          ? "merged"
          : task.status === "blocked" || task.status === "dep_blocked"
            ? "closed"
            : "";
    const bits = [`task ${task.id}`];
    if (task.lane) bits.push(`lane ${task.lane}`);
    if (task.progressStage) bits.push(task.progressStage);
    const more = earlier.length
      ? ` <span class="muted">(+${earlier.length} earlier task${earlier.length === 1 ? "" : "s"})</span>`
      : "";
    return ` <span class="badge ${cls}">queue: ${esc(task.status)}</span> <span class="muted">${esc(bits.join(" · "))}</span>${more}`;
  };
  const rows = steps
    .map((s) => {
      const stateBadge =
        s.liveState === null
          ? `<span class="badge">not resolved in the current source</span>`
          : `<span class="badge ${s.liveState === "open" ? "open" : "closed"}">issue ${esc(s.liveState)}</span>`;
      const box = s.checked ? "☑" : "☐";
      const done = s.doneNote ? `<br><span class="muted">${esc(s.doneNote)}</span>` : "";
      return `<div class="card"><strong>${box} ${esc(s.label)} — <a href="${esc(s.url)}">#${s.issueNumber}</a> ${esc(s.liveTitle ?? s.title)}</strong><br>${stateBadge}${queueBadge(s)}${done}</div>`;
    })
    .join("");
  return `${head}${sourceLine}${queueStale}${rows}`;
}

/** The Phase tracker panel (#9923): per-phase validation state for
 * Phases 0-13, derived live from the items source (the four #796
 * validation audits, #1075-#1078, already in the id project's tracked
 * refs). No hand-maintained values: a reopened audit or a new open
 * follow-up changes the panel without an edit. Phase 12 also shows
 * F-item progress counted from #1235's ordered step list, and Phases
 * 11 and 13 count the F-item mapping table in planning/roadmaps/id.md
 * (the source of record, #9923 step 3) when the source carries it;
 * Phases 0-10 joined that table for #13944 (F-numbering mirroring
 * their plans' P-step lists, refs = their validation audits), so
 * they derive from the same mapping instead of stating a gap. */
export function phaseTrackerPanelHtml(
  items: Map<RefKey, GhItem>,
  phase12: Phase12Snapshot | null = null,
  phaseMapping: PhaseMappingSnapshot | null = null,
): string {
  const umbrellaUrl = `https://github.com/${PHASE_TRACKER_REPO}/issues/${PHASE_TRACKER_UMBRELLA}`;
  const head = `<h2>Phase tracker — validation by phase, live from <a href="${umbrellaUrl}">#${PHASE_TRACKER_UMBRELLA}</a></h2>`;
  const progress = derivePhase12Progress(phase12, items);
  const mappedProgress = phaseMapping ? deriveMappedProgress(phaseMapping.entries, items) : null;
  const sourceLine = `<p class="muted">Source: the validation audits <a href="https://github.com/${PHASE_TRACKER_REPO}/issues/1075">#1075</a>–<a href="https://github.com/${PHASE_TRACKER_REPO}/issues/1078">#1078</a> under umbrella <a href="${umbrellaUrl}">#${PHASE_TRACKER_UMBRELLA}</a>, resolved from the same live GitHub items as the tracked-issues table above (the page provenance stamp applies). Phase 12 progress counts the ordered steps in <a href="${PHASE12_ISSUE_URL}">#1235</a> against live issue state${phase12 ? ` (body fetched ${esc(phase12.fetchedAt)})` : ""}; ${phaseMapping ? `All phases except 12 count the F-item mapping table in <a href="https://github.com/${PHASE_TRACKER_REPO}/blob/main/planning/roadmaps/id.md">planning/roadmaps/id.md</a> (read ${esc(phaseMapping.fetchedAt)}) against live item state — for Phases 0–10 the F-numbering mirrors their plans' P-step lists and each item's ref is its phase's validation audit (#1075–#1077); ` : "The F-item mapping is unavailable in the current source, so no mapped percentage is shown — a number is never invented."}</p>`;
  const rows = derivePhaseValidation(items)
    .map((r) => {
      const badge =
        r.state === "validated"
          ? `<span class="badge closed">validated — audit closed</span>`
          : r.state === "validation-open"
            ? `<span class="badge open">validation open — audit open</span>`
            : r.state === "unresolved"
              ? `<span class="badge">audit not resolved in the current source</span>`
              : `<span class="badge">no validation audit</span>`;
      const audit =
        r.auditIssue === null
          ? `<span class="muted">—</span>`
          : `<a href="${esc(r.auditUrl ?? "")}">#${r.auditIssue}</a>${r.auditTitle ? ` ${esc(r.auditTitle)}` : ""}`;
      const followUps = r.openFollowUps.length
        ? r.openFollowUps.map((f) => `<a href="${esc(f.url)}">#${f.issueNumber}</a>`).join(" ")
        : `<span class="muted">—</span>`;
      const mapped = mappedProgress?.get(r.phase);
      const progressCell =
        r.phase === 12
          ? progress
            ? `<strong>${progress.done}/${progress.total} steps done (${progress.percent}%)</strong><br><span class="muted">${progress.open} open${progress.unresolved ? ` · ${progress.unresolved} not resolved in the current source` : ""} — from <a href="${PHASE12_ISSUE_URL}">#1235</a></span>`
            : `<span class="muted">withheld — #1235 feed unavailable or stale</span>`
          : mapped
            ? `<strong>${mapped.done}/${mapped.total} items done (${mapped.percent}%)</strong><br><span class="muted">${mapped.open} open${mapped.unresolved ? ` · ${mapped.unresolved} not resolved in the current source` : ""} — from the F-item mapping in planning/roadmaps/id.md</span>`
            : `<span class="muted">not derived — F-item mapping unavailable in the current source</span>`;
      return `<tr><th scope="row">Phase ${r.phase}</th><td>${badge}</td><td>${audit}</td><td>${followUps}</td><td>${progressCell}</td></tr>`;
    })
    .join("");
  return `${head}${sourceLine}<div class="tblwrap"><table class="table table-zebra"><caption>Validation state per id-program phase, derived from live audit issue state</caption><thead><tr><th scope="col">Phase</th><th scope="col">Validation</th><th scope="col">Audit</th><th scope="col">Open follow-ups</th><th scope="col">Progress</th></tr></thead><tbody>${rows}</tbody></table></div>`;
}

/** The Build health panel (#9924): current CI lane states + the id
 * E2E nightly trend, projected from the Actions-API snapshot. A stale
 * or missing snapshot renders the warning INSTEAD of lane states —
 * silently old CI state is the defect #1239's rules name. A lane whose
 * read failed renders unavailable; a failing nightly run keeps its
 * link, which is the point of the trend. */
export function buildHealthPanelHtml(snapshot: BuildHealthSnapshot | null): string {
  const head = `<h2>Build health — live from the GitHub Actions API</h2>`;
  if (!snapshot) {
    return `${head}<div class="banner">The Actions API snapshot is unavailable in the current source, so no lane state is shown — this panel never renders hand-kept CI values.</div>`;
  }
  const sourceLine = `<p class="muted">Source: GitHub Actions API for ${esc(BUILD_HEALTH_REPO)}, fetched ${esc(snapshot.fetchedAt)}.</p>`;
  if (buildHealthIsStale(snapshot)) {
    return `${head}${sourceLine}<div class="banner">Stale source: this snapshot was fetched at ${esc(snapshot.fetchedAt)}, outside the freshness window — lane states are withheld rather than shown silently old. They refresh on the next successful Actions read.</div>`;
  }
  const runBadge = (status: string, conclusion: string | null): string => {
    if (status !== "completed") return `<span class="badge implementing">${esc(status)}</span>`;
    if (conclusion === "success") return `<span class="badge merged">success</span>`;
    if (conclusion === "failure") return `<span class="badge closed">failure</span>`;
    return `<span class="badge">${esc(conclusion ?? "completed")}</span>`;
  };
  const laneCards = snapshot.lanes
    .map((laneRuns) => {
      if (laneRuns.error) {
        return `<div class="card"><strong>${esc(laneRuns.lane.label)}</strong><br><span class="badge">unavailable</span><br><span class="muted">Actions read failed: ${esc(laneRuns.error)}</span></div>`;
      }
      const proj = projectLane(laneRuns);
      if (!proj.latest) {
        return `<div class="card"><strong>${esc(laneRuns.lane.label)}</strong><br><span class="badge">no runs reported</span></div>`;
      }
      const history = proj.history
        .map(
          (r) =>
            `<a href="${esc(r.url)}">${r.conclusion === "success" ? "✓" : r.conclusion === "failure" ? "✗" : "·"}</a>`,
        )
        .join(" ");
      return `<div class="card"><strong>${esc(laneRuns.lane.label)}</strong><br>${runBadge(proj.latest.status, proj.latest.conclusion)} <span class="muted">latest: <a href="${esc(proj.latest.url)}">${esc(proj.latest.createdAt)}</a> (${esc(proj.latest.event)})</span>${history ? `<br><span class="muted">recent completed: ${history}</span>` : ""}</div>`;
    })
    .join("");
  let nightlyHtml: string;
  if (snapshot.nightly.error) {
    nightlyHtml = `<div class="card"><strong>${esc(snapshot.nightly.lane.label)} — nightly trend</strong><br><span class="badge">unavailable</span><br><span class="muted">Actions read failed: ${esc(snapshot.nightly.error)}</span></div>`;
  } else {
    const trend = projectNightlyTrend(snapshot.nightly);
    nightlyHtml = trend.length
      ? `<div class="card"><strong>${esc(snapshot.nightly.lane.label)} — nightly trend (oldest → newest)</strong><br>${trend
          .map(
            (r) =>
              `<a href="${esc(r.url)}">${r.conclusion === "success" ? "✓" : r.conclusion === "failure" ? "✗" : "·"} ${esc(r.createdAt.slice(0, 10))}</a>`,
          )
          .join(" · ")}</div>`
      : `<div class="card"><strong>${esc(snapshot.nightly.lane.label)} — nightly trend</strong><br><span class="badge">no completed runs reported</span></div>`;
  }
  return `${head}${sourceLine}${laneCards}${nightlyHtml}`;
}

/** The Phase 12 readiness panel (#9926): F12.1–F12.18 with design
 * state (from #534's checklist), live issue state, and PR state
 * (timeline cross-references). The F-item list is parsed from
 * #1235's body — never hand-typed here. A stale snapshot withholds
 * the rows behind the warning; missing sub-feeds render their
 * column as unavailable instead of inventing a state. */
export function f12ReadinessPanelHtml(
  phase12: Phase12Snapshot | null,
  design: Phase12Snapshot | null,
  prRefs: Map<number, F12PrRef[]> | null,
  items: Map<RefKey, GhItem>,
): string {
  const head = `<h2>Phase 12 readiness — F12.1–F12.18, live</h2>`;
  if (!phase12) {
    return `${head}<div class="banner">The #1235 body feed is unavailable in the current source, so no F-item state is shown — the F-item list itself comes from that body, and this panel never renders a hand-typed copy.</div>`;
  }
  const sourceLine = `<p class="muted">Sources: F-item list + order from <a href="${PHASE12_ISSUE_URL}">developing-today/code#1235</a> (fetched ${esc(phase12.fetchedAt)}); design state from <a href="${F12_DESIGN_ISSUE_URL}">#534</a>${design ? ` (fetched ${esc(design.fetchedAt)})` : " (feed unavailable — design column withheld)"}; issue state is live GitHub state; PR state from issue-timeline cross-references${prRefs ? "" : " (timeline read unavailable — PR column withheld)"}.</p>`;
  if (phase12IsStale(phase12) || (design !== null && phase12IsStale(design))) {
    return `${head}${sourceLine}<div class="banner">Stale source: a body snapshot feeding this panel is outside the freshness window — the F-item rows are withheld rather than shown silently old. They refresh on the next successful body read.</div>`;
  }
  const fItems = parseF12Items(phase12.body);
  if (!fItems.length) {
    return `${head}${sourceLine}<div class="banner">No F-items could be parsed from the current #1235 body, so no readiness state is shown.</div>`;
  }
  const designs = design ? parseF12Designs(design.body) : new Map();
  const rows = projectF12Readiness(fItems, designs, items, prRefs ?? new Map())
    .map((r) => {
      const issueBadge =
        r.issueState === null
          ? `<span class="badge">issue not resolved in the current source</span>`
          : `<span class="badge ${r.issueState === "open" ? "open" : "closed"}">issue ${esc(r.issueState)}</span>`;
      const designBadge = !design
        ? `<span class="badge">design feed unavailable</span>`
        : r.design === null
          ? `<span class="badge">no design entry in #534</span>`
          : r.design.done
            ? `<span class="badge merged">design ${esc(r.design.designId)} done</span>`
            : `<span class="badge implementing">design ${esc(r.design.designId)} pending</span>`;
      const prBadge = !prRefs
        ? `<span class="badge">PR read unavailable</span>`
        : r.prs.length
          ? r.prs
              .map(
                (p) =>
                  `<span class="badge ${p.state === "merged" ? "merged" : p.state === "open" ? "open" : "closed"}"><a href="${esc(p.url)}">PR #${p.number} ${esc(p.state)}</a></span>`,
              )
              .join(" ")
          : `<span class="badge">no cross-referenced PR</span>`;
      return `<div class="card"><strong>${esc(r.f)} — <a href="${esc(r.url)}">#${r.issueNumber}</a> ${esc(r.title ?? "")}</strong><br>${designBadge} ${issueBadge} ${prBadge}</div>`;
    })
    .join("");
  return `${head}${sourceLine}${rows}`;
}

export function queuePanelHtml(snapshot: QueueSnapshot | null): string {
  const head = `<h2>Queue &amp; fleet \u2014 live from the fleet queue broker</h2>`;
  if (!snapshot) {
    return `${head}<div class="banner">The queue-broker snapshot is unavailable in the current source, so no queue state is shown \u2014 this panel never renders hand-kept queue values.</div>`;
  }
  const fetchedIso = new Date(snapshot.fetchedAt).toISOString();
  const historySource = snapshot.history ? " + /history-since" : "";
  const sourceLine = `<p class="muted">Source: queue-broker /health + /list${historySource} (read-only), fetched ${esc(fetchedIso)}.</p>`;
  if (!snapshot.available || snapshot.stale) {
    const why = snapshot.errors.length ? ` Last read error: ${esc(snapshot.errors[0])}.` : "";
    return `${head}${sourceLine}<div class="banner">Stale source: this queue snapshot was fetched at ${esc(fetchedIso)}, outside the freshness window \u2014 queue numbers are withheld rather than shown silently old.${why} They refresh on the next successful broker read.</div>`;
  }
  const age = (ms: number | null): string => {
    if (ms === null) return "\u2014";
    const mins = Math.floor(ms / 60_000);
    if (mins < 60) return `${mins}m`;
    const hours = Math.floor(mins / 60);
    if (hours < 48) return `${hours}h ${mins % 60}m`;
    return `${Math.floor(hours / 24)}d ${hours % 24}h`;
  };
  const streamRows = snapshot.queues
    .map(
      (q) =>
        `<tr><td>${esc(q.queue)}</td><td>${q.queued}</td><td>${q.claimed}</td><td>${q.parked}</td><td>${q.blocked}</td><td>${q.done}</td><td>${esc(age(q.oldestQueuedAgeMs))}</td><td>${q.terminalEvents24h} / ${q.terminalEvents7d}</td></tr>`,
    )
    .join("");
  const streams = `<table><caption>Queue streams</caption><thead><tr><th scope="col">Queue stream</th><th scope="col">Queued</th><th scope="col">Claimed</th><th scope="col">Parked</th><th scope="col">Blocked</th><th scope="col">Done</th><th scope="col">Oldest queued</th><th scope="col">Terminal 24h / 7d</th></tr></thead><tbody>${streamRows}</tbody></table>`;
  const idItems = snapshot.tasks.filter(
    (task) =>
      task.repo === "developing-today/code" &&
      task.issue !== null &&
      (task.status === "claimed" || task.status === "parked"),
  );
  const itemRows = idItems
    .slice(0, 50)
    .map((task) => {
      const who = [task.worker, task.lane].filter(Boolean).join(" / ") || "\u2014";
      const stage = task.progressStage ? esc(task.progressStage) : "\u2014";
      return `<tr><td>${task.id}</td><td><a href="https://github.com/developing-today/code/issues/${task.issue}">#${task.issue}</a> ${esc(task.title)}</td><td>${esc(task.status)}</td><td>${esc(who)}</td><td>${stage}</td></tr>`;
    })
    .join("");
  const items = idItems.length
    ? `<h3>Claimed &amp; parked id items (${idItems.length})</h3><table><caption>Claimed &amp; parked id items</caption><thead><tr><th scope="col">Task</th><th scope="col">Issue</th><th scope="col">Status</th><th scope="col">Worker / lane</th><th scope="col">Stage</th></tr></thead><tbody>${itemRows}</tbody></table>`
    : `<h3>Claimed &amp; parked id items</h3><p class="muted">None in the current snapshot.</p>`;
  const history = snapshot.history;
  let historyHtml = "";
  if (history) {
    if (!history.available) {
      historyHtml = `<h3>Recent task changes</h3><p class="muted">History feed unavailable at cursor ${history.cursor}: ${esc(history.error ?? "no error detail")}.</p>`;
    } else if (history.rows.length === 0) {
      historyHtml = `<h3>Recent task changes</h3><p class="muted">No new transitions after history cursor ${history.cursor}.</p>`;
    } else {
      const rows = [...history.rows]
        .sort((a, b) => b.id - a.id)
        .slice(0, 20)
        .map((event) => {
          const transition = `${event.fromStatus ?? "\u2014"} \u2192 ${event.toStatus ?? event.kind}`;
          return `<tr><td>${event.id}</td><td>${event.taskId}</td><td>${esc(event.kind)}</td><td>${esc(transition)}</td><td>${esc(event.actor ?? "\u2014")}</td><td>${esc(event.createdAt)}</td></tr>`;
        })
        .join("");
      const more = history.hasMore
        ? `<p class="muted">More history remains after cursor ${history.cursor}; the next read resumes there.</p>`
        : "";
      historyHtml = `<h3>Recent task changes</h3><table><caption>Task history after cursor ${history.cursor}</caption><thead><tr><th scope="col">History id</th><th scope="col">Task</th><th scope="col">Kind</th><th scope="col">Transition</th><th scope="col">Actor</th><th scope="col">At</th></tr></thead><tbody>${rows}</tbody></table>${more}`;
    }
  }
  return `${head}${sourceLine}${streams}${items}${historyHtml}`;
}

export function renderProgramProject(
  detail: ProjectDetail,
  items: Map<RefKey, GhItem>,
  provenance: Provenance,
  provDetail?: ProvenanceDetail,
  epicPool: ProjectPoolItem[] = detail.poolItems,
  phase12?: Phase12Snapshot | null,
  buildHealth?: BuildHealthSnapshot | null,
  f12?: { design: Phase12Snapshot | null; prRefs: Map<number, F12PrRef[]> | null } | null,
  queueSnap?: QueueSnapshot | null,
  phaseMapping?: PhaseMappingSnapshot | null,
  graphActions?: EpicGraphActionContext,
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
  if (detail.doc?.latestStatus) {
    const status = detail.doc.latestStatus;
    linkBits.push(
      `Latest status: <a href="${esc(status.url)}">${esc(status.title ?? status.dir)}</a> <span class="muted">(${esc(status.timestamp)} \u2014 newest <code>doc/</code> status record, derived at request time)</span>`,
    );
  } else if (detail.doc) {
    linkBits.push(
      `<span class="muted">No status record under <code>doc/</code> for this project yet \u2014 checked the code checkout at ${esc(detail.doc.fetchedAt)}.</span>`,
    );
  } else if (config.statusReport) {
    linkBits.push(`Status report: <a href="${esc(config.statusReport.url)}">${esc(config.statusReport.label)}</a>`);
  } else {
    linkBits.push(`<span class="muted">No status report yet for this project.</span>`);
  }
  if (detail.doc?.description) {
    linkBits.push(
      `Description derived from <a href="${esc(detail.doc.roadmapUrl)}">${esc(detail.doc.roadmapPath)}</a> <span class="muted">(read at request time)</span>`,
    );
  } else if (detail.doc) {
    linkBits.push(
      `<span class="muted">No prose description found in ${esc(detail.doc.roadmapPath)} \u2014 the registry intro is shown instead.</span>`,
    );
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
    poolHtml = `<p class="muted"><strong>${esc(weightedLine(allW))}</strong><br>${esc(handsLine(allW))}<br><strong>${detail.poolItems.length} items</strong> with Area <code>${esc(config.poolArea)}</code> — by status: ${countList(detail.poolByStatus)}. Full pool view: <a href="/projects?area=${encodeURIComponent(config.poolArea)}">/projects?area=${esc(config.poolArea)}</a>.</p>${epicHtml}${poolGraphHtml(detail.poolItems, allW)}${epicGraphSectionHtml(epicPool, detail.poolItems, graphActions)}${cards}${more}`;
  }

  let roadmapHtml: string;
  if (config.roadmapArea === null) {
    roadmapHtml = `<div class="banner">No Grist roadmap area yet — this project has no rows in the Program roadmap table. Its horizons live in its home doc and tracked issues.</div>`;
  } else if (!detail.roadmapRows.length) {
    roadmapHtml = `<div class="banner">No roadmap rows with area <code>${esc(config.roadmapArea)}</code> in the current source.</div>`;
  } else {
    const rollups = buildEpicRollups(epicPool);
    const rowRollups = new Map<string, EpicRollup>();
    for (const row of detail.roadmapRows) {
      const epic = row.links
        .map((link) => epicPool.find((item) => item.ref === link.ref && item.kind === "Epic"))
        .find(Boolean);
      const rollup = epic ? rollups.get(epic.ref) : undefined;
      if (rollup) rowRollups.set(row.subject, rollup);
    }
    roadmapHtml = `<p class="muted">${detail.roadmapRows.length} row${detail.roadmapRows.length === 1 ? "" : "s"} with area <code>${esc(config.roadmapArea)}</code> in Grist's Program roadmap. Timeline lanes and drill-downs use the shared roadmap renderer; epic rollups are per matched row.</p>${timelineTable(buildTimeline(detail.roadmapRows), items, undefined, undefined, undefined, undefined, undefined, undefined, { epicRollups: rowRollups, showRowNotes: true })}`;
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

  const phaseTrackerHtml =
    config.id === "id" ? phaseTrackerPanelHtml(items, phase12 ?? null, phaseMapping ?? null) : "";
  const phase12Html =
    config.id === "id" && phase12 !== undefined ? phase12PanelHtml(phase12, items, queueSnap ?? null) : "";
  const buildHealthHtml = config.id === "id" && buildHealth !== undefined ? buildHealthPanelHtml(buildHealth) : "";
  const f12Html =
    config.id === "id" && f12 !== undefined
      ? f12ReadinessPanelHtml(phase12 ?? null, f12?.design ?? null, f12?.prRefs ?? null, items)
      : "";
  const queueHtml = config.id === "id" && queueSnap !== undefined ? queuePanelHtml(queueSnap) : "";

  const ed = config.editorial;
  const editorialHtml = ed
    ? `${ed.done.length ? `<h2>What's built</h2>${programEntries(ed.done, items)}` : ""}${ed.next.length ? `<h2>What's next</h2>${programEntries(ed.next, items)}` : ""}${ed.longTerm.length ? `<h2>Long-term roadmap</h2>${programEntries(ed.longTerm, items)}` : ""}${ed.gaps.length ? `<h2>Known gaps</h2>${programEntries(ed.gaps, items)}` : ""}`
    : "";

  const body = `<h1>${esc(config.title)}</h1>
<p class="muted">${esc(detail.doc?.description ?? config.intro)}</p>
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
${phaseTrackerHtml}${phase12Html}${buildHealthHtml}${f12Html}${queueHtml}${editorialHtml}`;
  return layout(
    config.title,
    body,
    provenance,
    "/program",
    graphActions?.identities ? ACTION_SCRIPT : "",
    provDetail ? freshnessChips(provDetail, ["orchestration", "items", "roadmap"]) : "",
  );
}
