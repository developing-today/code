// render.ts split (#13051): today, queue and overview views.
// render.ts re-exports every public name from here; callers keep importing "./render.ts".

import { type AttentionView, type TodayObjective, type TodayView, buildQuotaActions, windowPercent } from "./derive.ts";
import type { DashAction } from "./actions.ts";
import { cardState } from "./card-state.ts";
import { AA_PLAN_FACTS } from "./aa.ts";
import { FEED_STALE_MS, ageMsText, feedAgeText } from "./feed.ts";
import { providerDisplayNames } from "./provider-display.ts";
import {
  reconciledInProgress,
  type InProgressSignal,
  type QueueView,
  type QueueRow,
  type ReconciledInProgressItem,
} from "./queue-view.ts";
import type { HealthView } from "./health.ts";
import {
  type BoardItem,
  type GhItem,
  type OrchestrationSection,
  type ProjectPoolItem,
  type Provenance,
  type ProvenanceDetail,
  type QuotaState,
  type QuotaView,
  type RefKey,
  type SourceCoverage,
  isValidRef,
  refUrl,
} from "./model.ts";

import {
  esc,
  fmtDate,
  FRAG_WATCHDOG_SCRIPT,
  freshnessChips,
  layout,
  sessionObjectRef,
  tierItemObjectRef,
  coverageHtml,
} from "./render-core.ts";

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

  const heartbeatOnly = quota.gatherer?.source === "standalone-heartbeat";
  const poolErrors = quota.pools.filter((pool) => pool.state === "error" || pool.freshness?.state === "error");
  const providerErrors = quota.gatherer?.providers.filter((provider) => provider.state === "error") ?? [];
  // A standalone tick is not a replacement quota measurement. Preserve base
  // pool failures and include heartbeat failures, deduplicating known services.
  const quotaErrors = heartbeatOnly
    ? new Set([...poolErrors.map((pool) => pool.id), ...providerErrors.map((provider) => provider.serviceId)]).size
    : quota.gatherer
      ? providerErrors.length
      : poolErrors.length;
  const quotaUnknown =
    quotaUnavailable || ((!quota.gatherer || heartbeatOnly) && quota.pools.length === 0 && quotaErrors === 0);
  const quotaState = quotaUnknown ? "unknown" : quotaErrors ? "warn" : quota.gatherer?.ok === false ? "unknown" : "ok";
  const quotaText = quotaUnknown
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
  questionsHtml = "",
): string {
  const walled = quota.pools.filter((p) => p.state === "walled");
  const quotaActions = buildQuotaActions(quota, AA_PLAN_FACTS);
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
  // #1174: the same eligible quota actions as /quota's strip and
  // /attention's orchestrator tier, derived once above — never a
  // second list. Walled pools without a typed action still banner.
  for (const a of quotaActions) {
    alerts.push(
      `<div class="banner"><strong>Quota action:</strong> ${esc(a.title)} — ${esc(a.detail)} <a href="${esc(a.href)}">quota</a></div>`,
    );
  }
  for (const p of walled.filter((p) => !quotaActions.some((a) => a.poolId === p.id))) {
    alerts.push(
      `<div class="banner"><strong>Walled pools:</strong> ${esc(`${displayPoolProvider(p.provider)} (${p.plan})`)} — <a href="/quota">quota</a></div>`,
    );
  }
  const wipViolations = (today.wipLanes ?? []).flatMap((lane) => [
    ...(lane.overCap ? [`${lane.lane} ${lane.count}/${lane.cap}`] : []),
    ...lane.projects
      .filter((scope) => scope.overCap)
      .map((scope) => `project ${scope.key} · ${lane.lane} ${scope.count}/${scope.cap}`),
    ...lane.epics
      .filter((scope) => scope.overCap)
      .map((scope) => `epic ${scope.key} · ${lane.lane} ${scope.count}/${scope.cap}`),
  ]);
  if (wipViolations.length) {
    alerts.push(
      `<div class="banner"><strong>WIP cap violations:</strong> ${wipViolations.map(esc).join("; ")} — <a href="/board">board</a>. Informational only; work is not blocked.</div>`,
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
  const quotaObjectives: TodayObjective[] = quotaActions.map((a) => ({
    title: a.title,
    detail: a.detail,
    chip: "quota · action",
  }));
  const allObjectives = [...today.objectives, ...quotaObjectives];
  // #6174: an objective ref comes from publisher lined-up data and is
  // only sometimes a GitHub ref — tracked-goal items carry their goal id,
  // and refUrl throws on those, which 500ed the whole /today render (the
  // Today-page sibling of the #6149 Needs-you fix). Mirror tierItemCard:
  // GitHub refs link, everything else renders as plain code.
  const objectives = allObjectives.length
    ? allObjectives
        .map(
          (o, i) =>
            `<div class="card obj"><div class="n">${i + 1}</div><div><strong>${esc(o.title)}</strong> <span class="badge src">${esc(o.chip)}</span>${o.detail ? `<br><span class="muted">${esc(o.detail)}</span>` : ""}${o.statusNote ? `<br><span class="badge stale" title="Source status vs derived status disagree in the canonical taxonomy (#1198) — both are shown, neither is preferred">${esc(o.statusNote)}</span>` : ""}${o.ref ? `<br>${isValidRef(o.ref) ? `<a href="${esc(refUrl(o.ref))}">${esc(o.ref)}</a>` : `<code>${esc(o.ref)}</code>`}` : ""}</div></div>`,
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
${nextUp}${questionsHtml}${actions}${coverageHtml(coverage)}`;
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
  questionsHtml = "",
): string {
  return layout(
    "Today",
    `<h1>Today</h1>${renderTodayBody(today, attention, quota, coverage, health, quotaUnavailable, queue, queueStale, questionsHtml)}`,
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
  questionsHtml = "",
): string {
  return layout(
    "Today",
    `<h1>Today</h1><noscript><meta http-equiv="refresh" content="0; url=/today?full=1"></noscript><div id="today-main" data-frag data-init="@get('/frag/today')" data-on:dashrefresh__window="@get('/frag/today')">${todayHealthStrip(health, quota, quotaUnavailable)}<div class="skel">Loading today…</div></div>${questionsHtml}`,
    provenance,
    "/today",
    FRAG_WATCHDOG_SCRIPT,
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
      const signals = item.signals?.length
        ? `<br>${item.signals
            .map(
              (signal) =>
                `<span class="badge ${signal === "open-pr" ? "open" : signal === "active-session" ? "orch" : "src"}">${signal === "project-status" ? "Project Status" : signal === "active-session" ? "Active session" : "Open PR"}</span>`,
            )
            .join(" ")}`
        : "";
      const clash = item.disagreement
        ? ` <span class="badge stale" title="The Project Status and the state the rest of the dashboard derives disagree — both are shown, neither is preferred">⚠ project: ${esc(item.disagreement.project)} · derived: ${esc(item.disagreement.derived)}</span>`
        : "";
      const horizon = item.horizon ? ` · ${esc(item.horizon)}` : "";
      const title = item.url ? `<a href="${esc(item.url)}">${esc(item.title)}</a>` : esc(item.title);
      return `<article class="card"><span class="badge">${esc(area)}</span> <span class="badge">${esc(item.status)}${horizon}</span><br><strong>${title}</strong>${item.ref ? `<br><span class="muted">${esc(item.ref)}</span>` : ""}<br><span class="muted">${esc(age)} · ${esc(item.source)}</span>${signals}${clash}${epic}${why}${blockers}${worker}</article>`;
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
    `<h1>Queue visibility</h1><noscript><meta http-equiv="refresh" content="0; url=/queue?full=1"></noscript><div id="queue-main" data-frag data-init="@get('/frag/queue')" data-on:dashrefresh__window="@get('/frag/queue')"><div class="skel">Loading queue…</div></div>`,
    provenance,
    "/queue",
    FRAG_WATCHDOG_SCRIPT,
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

export type OverviewInProgressSignal = InProgressSignal;

export type OverviewInProgressItem = ReconciledInProgressItem;

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
  // #1116 item (5): when the feed itself has crossed the staleness
  // threshold the dashboard must say so visibly — a "feed X old" suffix
  // is not an actionable warning. The data-signal hook is what the
  // age-feed drill greps for as its rendered-page evidence.
  const feedAgeMs = summary.health.feedAgeMs;
  const feedStaleBanner =
    feedAgeMs !== null && feedAgeMs >= FEED_STALE_MS
      ? `<div class="banner feed-stale" data-signal="feed-stale">Feed stale — last collection ${esc(ageMsText(feedAgeMs))} ago (threshold ${esc(ageMsText(FEED_STALE_MS))}); the collector may be down. <a href="/health">Check health</a></div>`
      : "";
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
<h2>Health</h2>${feedStaleBanner}${healthStrip}
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
    `<h1>Overview</h1><noscript><meta http-equiv="refresh" content="0; url=/?full=1"></noscript><div id="overview-main" data-frag data-init="@get('/frag/overview')" data-on:dashrefresh__window="@get('/frag/overview')"><div class="skel">Loading the overview…</div></div>`,
    provenance,
    "/",
    FRAG_WATCHDOG_SCRIPT,
    detail ? freshnessChips(detail, ["board", "quota", "attention"]) : "",
  );
}
