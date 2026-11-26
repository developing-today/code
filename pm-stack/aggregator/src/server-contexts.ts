// Page/API context assemblies: the cached board, roadmap, today,
// overview, attention, program and health assemblies the read routes
// render from. Split out of server.ts (#13052).
import { stat } from "node:fs/promises";
import { type DashAction } from "./actions.ts";
import { effectiveNeedsOrder } from "./tier-items.ts";
import type { SavedView, ViewFilters, ViewItemLike } from "./views.ts";
import { filtersFromQuery, ownerKey } from "./views.ts";
import { createWatchdogGapSyncQueue, watchdogDeliveries } from "./activity.ts";
import { loadFeed } from "./feed.ts";
import { AA_PLAN_FACTS } from "./aa.ts";
import { shouldFetchNeedsDrewCensus, type NeedsDrewCensus } from "./needs-drew.ts";
import {
  buildHealth,
  resolveBuildVersion,
  summarizeItemCoverage,
  type HealthView,
  type ItemsCoverage,
} from "./health.ts";
import { computeEditionDrift, type EditionDrift } from "./edition-drift.ts";
import { join } from "node:path";
import { watchdogTierItems, WATCHDOG_PROBE_SERVICE_IDS } from "./watchdog.ts";
import { readHostHealthSnapshot } from "./host-health.ts";
import { allRefs, buildAttention, buildQuotaActions, deriveToday, quotaActionTierItems } from "./derive.ts";
import { reconcileRoadmap, unavailableRoadmapReconciliation, type RoadmapReconciliation } from "./roadmap-reconcile.ts";
import type { ProjectPoolItem, RefKey } from "./model.ts";
import { isValidRef } from "./model.ts";
import { programRefs } from "./program.ts";
import { renderBoard, deriveOverview, renderOverview, buildRoadmapDrill } from "./render.ts";
import { buildBriefs } from "./brief.ts";
import { buildDigest, type Digest, type DigestKind } from "./digest.ts";
import { searchWork } from "./search-view.ts";
import type { Source } from "./source.ts";
import { buildQueueView, queueInputsStale } from "./queue-view.ts";
import { TodayCache, TODAY_CACHE_TTL_MS } from "./today-cache.ts";
import { boardJsonBody, boardJsonViewBody } from "./board-json.ts";
import { groupSamplesByFacet } from "./slo-journal.ts";
import { SLO_DEFINITIONS, summarizeSloFacet, type SloFacetSummary } from "./slo.ts";
import { watchdogHeartbeatLine } from "./watchdog-heartbeat.ts";
import { createWatchdogHeartbeatRecorder, finishWatchdogGapSync } from "./watchdog-runner.ts";
import { quotaForecastContract, subjectForecastContracts, type ForecastContract } from "./forecast-contract.ts";
import {
  cfg,
  itemHistoryStore,
  sloJournal,
  actionsStore,
  commentsStore,
  viewsStore,
  freshnessStore,
  freshnessReconciler,
  activityStore,
  recordActivity,
  watchdogEpisodes,
  source,
  briefCommentsFor,
  assembleConflicts,
  commentFormOpts,
  emptyQuotaView,
} from "./server-wiring.ts";
import { requireHome } from "./home.ts";

export async function loadBoardContext() {
  const board = await source.board().catch(() => []);
  const refs = allRefs(board);
  const items = await source.items(refs).catch(() => new Map());
  return { board, items };
}

// #5082: the board assembly is the stack's most expensive read — a
// Kaneo login + poll with per-project task pagination, one projector
// /links call per card, and the items() closure over every ref — and
// every board-family request used to pay it in full. Under ordinary
// fleet load the assembly outlives the 10 s dashboard deadline
// (assembly-deadline.ts), so /board, /api/board.json, and the overview
// (which embeds the board) 503'd even with every upstream healthy;
// concurrent cold loads each started their own assembly and multiplied
// the damage (measured 2026-11-08: 3 concurrent -> 3 assemblies, all
// 503). The assembly now gets the /today contract (#1179): a short TTL
// serves warm hits from the last successful read, and concurrent cold
// requests share one in-flight load.
//
// The cache sits BELOW the variant split, which is what makes it safe
// for every consumer: ?done=all, saved-view filters, and the HTML/JSON
// renderers are all derived per request from this one context
// (boardJsonBody / boardJsonViewBody / renderBoard), so no variant can
// ever be served another variant's body. Provenance stays truthful the
// same way it does for /today: prov()/provDetail() are still read live
// per request at the call sites. Fail-soft fallbacks (an empty board
// when Kaneo is down) are the honest degraded state of the read and
// are cached as-is until expiry, exactly as #1179 documents; a
// throwing load is never cached.
export const boardContextCache = new TodayCache<Awaited<ReturnType<typeof loadBoardContext>>>(TODAY_CACHE_TTL_MS);
export async function boardContext(): Promise<Awaited<ReturnType<typeof loadBoardContext>>> {
  return boardContextCache.get(loadBoardContext);
}

/** All shared forecast contracts (#3984): epic-completion + stall-risk
 * per epic subject from the stored item history (#3981), and one
 * quota-exhaustion contract per quota pool. The rate for a pool is
 * only usable when the gatherer measured a token burn for a token
 * window with ok freshness — anything else stays the contract's
 * first-class quota_rate_unavailable state, never an invented date. */
export async function allForecastContracts(
  pool: ProjectPoolItem[],
): Promise<{ byRef: Map<RefKey, ForecastContract[]>; contracts: ForecastContract[] }> {
  const byRef = new Map<RefKey, ForecastContract[]>();
  const contracts: ForecastContract[] = [];
  const add = (c: ForecastContract) => {
    contracts.push(c);
    byRef.set(c.subjectRef, [...(byRef.get(c.subjectRef) ?? []), c]);
  };
  const cutoffs = await itemHistoryStore.readAll().catch(() => []);
  if (cutoffs.length) {
    const poolByRef = new Map(pool.map((it) => [it.ref, it]));
    const subjects = new Set<RefKey>();
    for (const cutoff of cutoffs) {
      for (const item of cutoff.items) {
        if (item.parentRef) subjects.add(item.parentRef);
      }
    }
    for (const subjectRef of subjects) {
      const poolItem = poolByRef.get(subjectRef);
      const statusDisagreement =
        poolItem &&
        poolItem.status &&
        ((poolItem.state === "closed" && poolItem.status !== "Done") ||
          (poolItem.state === "open" && poolItem.status === "Done"))
          ? `GitHub state ${poolItem.state} disagrees with Project Status ${poolItem.status}; both are shown, neither is preferred`
          : null;
      for (const c of subjectForecastContracts(cutoffs, subjectRef, {
        updatedAt: poolItem?.updatedAt ?? null,
        statusDisagreement,
        scope: `direct native children of ${subjectRef} in the Project #4 pool item history (curated pool scope, not a repo-wide backlog)`,
      })) {
        add(c);
      }
    }
  }
  const quota = await source.quota().catch(() => emptyQuotaView());
  const asOf = new Date().toISOString();
  for (const qpool of quota.pools) {
    const window = qpool.windows
      .filter((w) => w.limit !== null && w.limit > 0)
      .sort((a, b) => (b.percent ?? 0) - (a.percent ?? 0))[0];
    const rate =
      window && window.unit === "tokens" && qpool.burnTokensPerMin != null && qpool.freshness?.state === "ok"
        ? { amountPerHour: qpool.burnTokensPerMin * 60, observedAt: qpool.freshness.measuredAt }
        : null;
    add(
      quotaForecastContract({
        poolId: qpool.id,
        provider: qpool.provider,
        asOf,
        unit: window?.unit ?? "tokens",
        remainingAmount: window ? window.limit! - window.used : null,
        limitAmount: window?.limit ?? null,
        nextReset: window?.resetsAt ?? null,
        rate,
        scope: `quota pool ${qpool.id} (${qpool.provider}/${qpool.plan})`,
      }),
    );
  }
  return { byRef, contracts };
}

export async function roadmapContext() {
  const rows = await source.roadmap().catch(() => []);
  const refs = allRefs(rows);
  const [items, poolResult] = await Promise.all([
    source.items(refs).catch(() => new Map()),
    source.projects().then(
      (pool) => ({ pool, failed: false }),
      () => ({ pool: [] as ProjectPoolItem[], failed: true }),
    ),
  ]);
  // #1144: reconcile the Grist rows against the Project's Kind=Epic
  // pool items on every roadmap read, so the page flag and the JSON
  // drift report always describe the same pass as the rows themselves.
  // Codex P2 on PR 2428: an unread source is NOT an empty source — in
  // live mode, a Grist fixtures fallback or a fleet feed with no
  // projects section means reconciliation is unavailable, and no
  // drift may be asserted from it.
  let reconciliation: RoadmapReconciliation;
  if (cfg.source === "live") {
    const reasons: string[] = [];
    const roadmapProv = source.provenance().roadmap;
    if (!roadmapProv.startsWith("live")) reasons.push(`Grist roadmap read unavailable (${roadmapProv})`);
    if (poolResult.failed) reasons.push("Project pool read failed");
    const feed = await loadFeed(cfg.feedFile).catch(() => null);
    if (!feed) reasons.push("fleet feed unreadable — Project pool availability unknown");
    else if (feed.projects === undefined)
      reasons.push("fleet feed carries no projects section — Project pool availability unknown");
    reconciliation = reasons.length
      ? unavailableRoadmapReconciliation(reasons.join("; "))
      : reconcileRoadmap(rows, poolResult.pool);
  } else {
    reconciliation = reconcileRoadmap(rows, poolResult.pool);
  }
  // #1168: the drill-down index rides the same pass as the
  // reconciliation, and only exists when the reconciliation is
  // available — an unavailable source must not render as "no epic yet".
  const drill =
    reconciliation.availability === "available" ? buildRoadmapDrill(rows, reconciliation, poolResult.pool) : undefined;
  return { rows, items, pool: poolResult.pool, reconciliation, drill };
}

type SearchTimings = Record<string, number>;

export async function withSearchTiming<T>(
  timings: SearchTimings,
  stage: string,
  run: () => T | Promise<T>,
): Promise<T> {
  const started = performance.now();
  try {
    return await run();
  } finally {
    timings[stage] = performance.now() - started;
  }
}

export function withSearchTimings(response: Response, timings: SearchTimings): Response {
  const value = Object.entries(timings)
    .map(([stage, duration]) => `${stage};dur=${Math.max(0, duration).toFixed(2)}`)
    .join(", ");
  response.headers.set("server-timing", value);
  return response;
}

export async function searchContext(query: string) {
  const timings: SearchTimings = {};
  // #1212 slice 2: registry, DNS, services, and tier items join the
  // board/roadmap subjects. Each extra source fails soft (empty on
  // error) so one broken source never takes search down with it. The
  // source timings below overlap: these reads intentionally run in parallel.
  const [board, rows, registry, dns, usage, orchestration] = await Promise.all([
    withSearchTiming(timings, "board", () => source.board()),
    withSearchTiming(timings, "roadmap", () => source.roadmap()),
    withSearchTiming(timings, "registry", () => source.registry().catch(() => [])),
    withSearchTiming(timings, "dns", () => source.dns().catch(() => null)),
    withSearchTiming(timings, "usage", () => source.usage().catch(() => null)),
    withSearchTiming(timings, "orchestration", () => source.orchestration().catch(() => null)),
  ]);
  const tierItems = orchestration ? [...orchestration.tiers.needsDrew, ...orchestration.tiers.needsOrchestrator] : [];
  // Codex P2 on PR #7331: refs carried only by the slice-2 sources
  // (registry taskRefs, tier-item refs) join the resolution set, or
  // their search badges always render unresolved even when the live
  // pool/projector could resolve them.
  const extraRefs = [...registry.flatMap((record) => record.taskRefs), ...tierItems.map((item) => item.ref)].filter(
    isValidRef,
  );
  const refs = [...new Set([...allRefs(board), ...allRefs(rows), ...extraRefs])];
  const items = await withSearchTiming(timings, "items", () => source.items(refs));
  const extras = { registry, dns, services: usage?.services ?? [], tierItems };
  const results = await withSearchTiming(timings, "search", () => searchWork(board, rows, items, query, extras));
  return { query, results, items, timings };
}

// #3321: watchdog gap-sync passes run whole and in observation
// order through this queue (Codex P2, PR #5141 round 5) — see
// createWatchdogGapSyncQueue in activity.ts for why the bare
// fire-and-forget sync interleaved.
export const enqueueWatchdogGapSync = createWatchdogGapSyncQueue();
export const watchdogHeartbeatRecorder = createWatchdogHeartbeatRecorder(async (nowMs) => {
  await sloJournal.append([watchdogHeartbeatLine(nowMs)]);
});

/** Record liveness only after a watchdog derivation completes. A timer here
 * would stay green even if the watchdog work itself stopped making progress. */
export async function recordWatchdogHeartbeat(nowMs: number, scheduled = false): Promise<void> {
  try {
    await (scheduled ? watchdogHeartbeatRecorder.scheduled(nowMs) : watchdogHeartbeatRecorder.request(nowMs));
  } catch (error) {
    console.error(
      `[watchdog] WARNING: heartbeat append failed: ${error instanceof Error ? error.message : String(error)}`,
    );
  }
}

// #1148: the watchdog derives NEEDS ORCHESTRATOR items from the
// current diagnoses (feed age + probes via the health assembly —
// its probe cache keeps this cheap — plus the orchestration
// diagnosis and registry facet age). Live mode only: fixtures mode
// has no real inputs to watch. Derived on attention reads and on the
// server-owned watchdog tick below, so it does not depend on dashboard
// traffic. Recovery clears the cards on the next derivation. The health
// assembly here is deliberately UNPRIMED (see healthContext): the
// watchdog needs only the feed file, the cached probes, and the
// registry age from the source's already-loaded feed.
export async function deriveWatchdogItems(
  diagnosis?: ReturnType<NonNullable<Source["diagnostics"]>>["orchestration"],
  mode: "request" | "scheduled" = "request",
) {
  if (cfg.source !== "live") return [];
  const [health, projectAudit, hostHealth] = await Promise.all([
    healthContext(false),
    source.projectAudit ? source.projectAudit().catch(() => null) : Promise.resolve(null),
    // #10141: the host-health collector's snapshot (claimant mode +
    // PM unit states). A read failure yields null, which the watchdog
    // surfaces as the snapshot-absent item — the surfacer's own
    // silence is a breach, never a pass.
    readHostHealthSnapshot(cfg.hostHealthFile).catch(() => null),
  ]);
  const items = watchdogTierItems({
    now: Date.now(),
    feedAgeMs: health.feed.ageMs,
    feedGeneratedAt: health.feed.generatedAt,
    ...(diagnosis ? { orchestration: diagnosis } : {}),
    projectAudit,
    ...(source.provenanceDetail ? { registryAgeMs: source.provenanceDetail().registry.ageMs } : {}),
    probes: health.services
      .filter((s) => WATCHDOG_PROBE_SERVICE_IDS.includes(s.def.id))
      .map((s) => ({
        id: s.def.id,
        name: s.def.name,
        ok: s.ok,
        detail: s.detail,
        ...(s.probedAt ? { probedAt: s.probedAt } : {}),
      })),
    hostHealth,
  });
  // #3321: every derivation is also an ingestion pass — the current
  // breach episodes go to the stream (dedupe makes re-derivation a
  // no-op) and the gap records sync with the breach set. Episodes
  // are stabilized first (first-observed `since` wins until the
  // breach clears), so one continuous outage is one delivery.
  const stableItems = watchdogEpisodes.stabilize(items);
  void recordActivity(watchdogDeliveries(stableItems));
  await finishWatchdogGapSync({
    items,
    sync: enqueueWatchdogGapSync(activityStore, stableItems),
    mode,
    recordHeartbeat: () => recordWatchdogHeartbeat(Date.now(), mode === "scheduled"),
    onError: (err) => {
      console.error(
        `[activity] WARNING: watchdog gap sync failed: ${err instanceof Error ? err.message : String(err)}`,
      );
    },
  });
  return items;
}

export async function loadAttentionContext(opts: { census?: boolean } = {}) {
  // #1226/#3970: the Needs-Drew census costs up to eight sequential
  // GitHub search calls (up to ~64 s when the API is unreachable). Only
  // the attention page render consumes it, so it opts in; the overview,
  // Today, /api/attention.json and /api/coverage.json must not inherit
  // that external dependency (Codex P2 on PR #3970).
  const wantsCensus = shouldFetchNeedsDrewCensus(opts);
  const [feed, summary, coverage, census] = await Promise.all([
    source.attention().catch(() => []),
    source.orchestration().catch(() => null),
    source.coverage().catch(() => []),
    // A source without the census (or a failed sweep) yields null and
    // the view renders it as unavailable, never as an empty queue.
    wantsCensus
      ? ((source.needsDrew?.() ?? Promise.resolve(null)).catch(() => null) as Promise<NeedsDrewCensus | null>)
      : Promise.resolve<NeedsDrewCensus | null>(null),
  ]);
  // Read the diagnosis AFTER orchestration() has run: the live source
  // recomputes it on every orchestration read (#1139).
  const diagnosis = source.diagnostics?.().orchestration;
  const watchdog = await deriveWatchdogItems(diagnosis);
  const quota = await source.quota().catch(() => null);
  const view = buildAttention(feed, summary, Date.now(), diagnosis, watchdog);
  // #1174: the same eligible quota actions as /quota's strip and
  // /today, as orchestrator-tier task items — derived once here from
  // the same pure function, so entries clear on every surface the
  // moment their source condition no longer holds.
  const quotaItems = quota ? quotaActionTierItems(buildQuotaActions(quota, AA_PLAN_FACTS)) : [];
  if (quota) {
    view.orchestratorExtra = [...view.orchestratorExtra, ...quotaItems];
  }
  return {
    view,
    quotaItems,
    coverage,
    orchestration: summary,
    diagnosis,
    census,
  };
}

// #9850: /api/attention.json is a sentinel-probed health surface, but
// the assembly above re-reads every source leg on every request — under
// load that per-request cost is what flapped the feed (multi-second
// answers at load ~10). Same contract as the today/overview assemblies
// (#5082): a short-TTL cache with coalesced loads and
// stale-while-revalidate, one cache per census variant — the census
// assembly costs up to ~64 s of GitHub search calls and must never
// ride the JSON path, or vice versa.
export const attentionContextCache = new TodayCache<Awaited<ReturnType<typeof loadAttentionContext>>>(
  TODAY_CACHE_TTL_MS,
);
export const attentionCensusContextCache = new TodayCache<Awaited<ReturnType<typeof loadAttentionContext>>>(
  TODAY_CACHE_TTL_MS,
);
export async function attentionContext(opts: { census?: boolean } = {}) {
  return opts.census
    ? attentionCensusContextCache.get(() => loadAttentionContext(opts))
    : attentionContextCache.get(() => loadAttentionContext(opts));
}

/**
 * One Datastar patch-elements event (#1444). The fragment root carries
 * its own id + `data-on:dashrefresh__window` refresh hook, so "outer"
 * mode replaces the shell's placeholder with a self-refreshing
 * container. The payload deliberately carries NO `data-init` (#10605):
 * an outer-mode patch re-mounts the element, so an init trigger inside
 * the payload would fetch itself again on every patch — a loop. First
 * paint is driven by the shell placeholder's `data-init` instead.
 */
export function ssePatch(selector: string, elementHtml: string): string {
  const lines = elementHtml
    .split("\n")
    .map((l) => `data: elements ${l}`)
    .join("\n");
  return `event: datastar-patch-elements\ndata: mode outer\ndata: selector ${selector}\n${lines}\n\n`;
}

export function sseResponse(body: string): Response {
  return new Response(body, {
    headers: { "content-type": "text/event-stream; charset=utf-8", "cache-control": "no-cache" },
  });
}

/** The render inputs the board page and its fragment share. */
export async function boardRender() {
  const { board, items } = await boardContext();
  const [pool, orchestration, formOpts, actionsByObject, counts] = await Promise.all([
    source.projects().catch(() => []),
    source.orchestration().catch(() => null),
    commentFormOpts(),
    actionsStore.byObject(),
    commentsStore.countsDetailed(),
  ]);
  const poolByRef = new Map(pool.map((p) => [p.ref, p]));
  const refs = [
    ...new Set(board.flatMap((item) => item.links.map((link) => link.ref)).filter((ref) => poolByRef.has(ref))),
  ];
  const comments = refs.length ? await briefCommentsFor(refs) : [];
  const orchestrationStatus = source.diagnostics?.().orchestration.state;
  const briefs = buildBriefs(pool, { comments, orchestration, orchestrationStatus });
  const act = {
    actionsByObject,
    poolByRef,
    identities: formOpts.identities,
    briefs,
  };
  return { board, items, counts, act };
}

/** The render inputs the attention page and its fragments share:
 *  the view, the action/comment stores grouped by object, and the
 *  effective Needs-you order preference (#1456). */
export async function attentionRender() {
  const { view, coverage, orchestration, diagnosis, census } = await attentionContext({ census: true });
  const [pool, formOpts, actionsByObject, commentsByObject, counts, allActions] = await Promise.all([
    source.projects().catch(() => []),
    commentFormOpts(),
    actionsStore.byObject(),
    commentsStore.byObject(),
    commentsStore.countsDetailed(),
    actionsStore.all(),
  ]);
  const order = effectiveNeedsOrder(allActions);
  const refs = [
    ...new Set([...view.drewExtra, ...view.orchestratorExtra].flatMap((item) => (item.ref ? [item.ref] : []))),
  ];
  const comments = refs.length ? await briefCommentsFor(refs) : [];
  const briefs = buildBriefs(pool, { comments, orchestration, orchestrationStatus: diagnosis?.state });
  const act = {
    actionsByObject,
    identities: formOpts.identities,
    commentsByObject,
    commentOpts: formOpts,
    needsOrder: order.order,
    needsOrderPending: order.pending,
  };
  return { view, coverage, counts, act, briefs, census };
}

export async function loadTodaySignals() {
  const [health, quotaResult] = await Promise.all([
    healthContext(false).catch(() => null),
    source
      .quota()
      .then((quota) => ({ quota, unavailable: false }))
      .catch(() => ({ quota: emptyQuotaView(), unavailable: true })),
  ]);
  return { health, quota: quotaResult.quota, quotaUnavailable: quotaResult.unavailable };
}

// #1179: the shell (/today) and the fragment (/frag/today) both need
// the health+quota strip, seconds apart on every page view. Bound the
// repeated live reads with a short TTL; concurrent cold requests share
// the in-flight load. Provenance stays truthful: prov()/provDetail()
// are still read live per request at the call sites below.
export const todaySignalsCache = new TodayCache<Awaited<ReturnType<typeof loadTodaySignals>>>(TODAY_CACHE_TTL_MS);
export async function todaySignals(): Promise<Awaited<ReturnType<typeof loadTodaySignals>>> {
  return todaySignalsCache.get(loadTodaySignals);
}

export async function loadTodayContext() {
  const [{ board, items }, { rows, pool }, { view, coverage, orchestration }, signals] = await Promise.all([
    boardContext(),
    roadmapContext(),
    attentionContext(),
    todaySignals(),
  ]);
  const diagnosis = source.diagnostics?.().orchestration;
  const queue = buildQueueView({ board, pool, items, orchestration });
  return {
    today: deriveToday(board, rows, items, orchestration, new Date(), diagnosis, pool),
    view,
    coverage,
    ...signals,
    queue,
    queueStale: queueInputsStale(signals.health),
  };
}

// #1179: /today?full=1, /frag/today, and /api/today.json all render
// from this one assembly. Same short-TTL + coalescing contract as the
// signals cache above; a throwing cold load is never cached (the
// fetch error boundary still 500s) and expiry serves the stale copy
// while one background refresh re-reads (#5082 SWR).
export const todayContextCache = new TodayCache<Awaited<ReturnType<typeof loadTodayContext>>>(TODAY_CACHE_TTL_MS);
export async function todayContext(): Promise<Awaited<ReturnType<typeof loadTodayContext>>> {
  return todayContextCache.get(loadTodayContext);
}

export async function queueContext() {
  const [{ board, items }, pool, orchestration] = await Promise.all([
    boardContext(),
    source.projects().catch(() => [] as ProjectPoolItem[]),
    source.orchestration().catch(() => null),
  ]);
  return buildQueueView({ board, pool, items, orchestration });
}

/** The render inputs the overview (front door, #1134) assembles:
 *  the attention view, the Project pool, quota, the health view, and
 *  the board — the same contexts the dedicated pages render from. */
export async function loadOverviewContext() {
  const [{ view }, pool, quota, health, { board, items }, orchestration, actionsByObject] = await Promise.all([
    attentionContext(),
    source.projects().catch(() => [] as ProjectPoolItem[]),
    source.quota().catch(() => emptyQuotaView()),
    healthContext(),
    boardContext(),
    source.orchestration().catch(() => null),
    actionsStore.byObject().catch(() => new Map()),
  ]);
  return { attention: view, pool, quota, health, board, items, orchestration, actionsByObject };
}

// #5082: same contract as the board assembly above — the overview
// fan-out (attention + pool + quota + health + board + orchestration)
// was re-run live on every /, /frag/overview, and /api/overview.json
// request and tripped the same 10 s deadline. The per-request
// derivations (deriveOverview / renderOverview) and the provenance
// reads stay live; only the assembly is cached. The board leg inside
// rides the boardContext cache, so a cold overview and a cold board
// share one board assembly.
export const overviewContextCache = new TodayCache<Awaited<ReturnType<typeof loadOverviewContext>>>(TODAY_CACHE_TTL_MS);
export async function overviewContext(): Promise<Awaited<ReturnType<typeof loadOverviewContext>>> {
  return overviewContextCache.get(loadOverviewContext);
}

// #3976 (Codex review on PR #9193): a freshness notice means canonical
// data changed, but the TTL caches above could still serve an assembly
// built before the notice for up to 10s — and nothing re-requests when
// the TTL expires, so an accepted revision could stay invisible until
// a later notice or a manual refresh. Invalidate the two assemblies
// the freshness fragments reread (/frag/overview, /frag/today) the
// moment a notice becomes durable. The store seam covers every
// acceptance path — in-process recordFreshness, the IPC handler, and
// the reconciler's synthetic invalidations all publish through the
// same store.
//
// #5082 (2026-11-14): invalidate, don't clear. Notices land every few
// minutes (115 in the 3 h before steward run 379), and clear() dropped
// the cached assembly entirely, so the first post-notice request paid
// the full cold assembly on the request path. That request is
// routinely an hourly watch arriving after a quiet gap; when the
// assembly chain inflated past the 10 s dashboard deadline under load
// (steward runs 378/379: /api/overview.json 503 at 10.42 s, pass 2
// clean), the deadline's last-good was always older than its 5-minute
// staleness cap at that cadence, so the fallback was a hard 503.
// invalidate() keeps the last assembly servable under the ordinary
// SWR contract — the first post-notice request serves it immediately
// and refreshes behind it — and the kick below starts that refresh
// at notice time, so an accepted revision becomes visible one
// assembly later without any request paying for the rebuild.
freshnessStore.onNotice(() => {
  overviewContextCache.invalidate();
  todayContextCache.invalidate();
  attentionContextCache.invalidate();
  attentionCensusContextCache.invalidate();
  void overviewContext().catch(() => {});
  void todayContext().catch(() => {});
});

/** The live-source half of the /program assembly (#1046 + #1121): the
 *  items closure for the program refs, the Project pool, the roadmap,
 *  orchestration, and the coverage census inputs. Everything
 *  view-dependent (saved-view filtering, briefs, forecasts, the
 *  render) is derived per request from this bundle in the handler. */
export async function loadProgramSources() {
  const [items, pool, rows, orchestration, census, poolAvailable] = await Promise.all([
    source.items(programRefs()).catch(() => new Map()),
    source.projects().catch(() => []),
    source.roadmap().catch(() => []),
    source.orchestration().catch(() => null),
    source.coverageCensus ? source.coverageCensus().catch(() => null) : Promise.resolve(null),
    source.projectsAvailable ? source.projectsAvailable().catch(() => false) : Promise.resolve(true),
  ]);
  return { items, pool, rows, orchestration, census, poolAvailable };
}

// #5082: /program is a bounded dashboard route with the same failure
// shape — its source fan-out ran live on every request. Same TTL +
// single-flight contract as the board/overview assemblies above.
export const programSourcesCache = new TodayCache<Awaited<ReturnType<typeof loadProgramSources>>>(TODAY_CACHE_TTL_MS);
export async function programSources(): Promise<Awaited<ReturnType<typeof loadProgramSources>>> {
  return programSourcesCache.get(loadProgramSources);
}

// -- Saved views (#1276) -------------------------------------------------
// A request names a view three ways, in precedence order: explicit
// vf.* filter params (the shareable-URL form — reproduces the view for
// any viewer with no store lookup), a view id (store lookup), or the
// viewer's own default view (the viewer is named by the `viewer` query
// param, the dashboard author key "<kind>:<id>"). No view named = the
// pages render exactly as before.

export interface ResolvedView {
  filters: ViewFilters;
  view: SavedView | null;
}

export async function resolveView(url: URL): Promise<ResolvedView | null> {
  const fromQuery = filtersFromQuery(url.searchParams);
  if (fromQuery) {
    const id = url.searchParams.get("view");
    return { filters: fromQuery, view: id ? await viewsStore.get(id).catch(() => null) : null };
  }
  const id = url.searchParams.get("view");
  if (id) {
    const view = await viewsStore.get(id).catch(() => null);
    return view ? { filters: view.filters, view } : null;
  }
  const viewer = url.searchParams.get("viewer");
  if (viewer) {
    const view = await viewsStore.defaultFor(viewer).catch(() => null);
    return view ? { filters: view.filters, view } : null;
  }
  return null;
}

export function poolItemLike(item: ProjectPoolItem): ViewItemLike {
  return {
    ref: item.ref,
    repo: item.repo,
    area: item.area,
    horizon: item.horizon,
    owner: item.author ?? null,
    state: item.state,
    epic: item.epicRef ?? null,
    status: item.status,
    kind: item.kind,
  };
}

export function eventContextByRef(
  pool: readonly ProjectPoolItem[],
): Map<string, { project: string; epic: string | null }> {
  const context = new Map<string, { project: string; epic: string | null }>();
  for (const item of pool) {
    const ref = item.ref.replace(/^gh:/i, "").toLowerCase();
    context.set(ref, {
      project: item.projectUrl,
      epic: item.epicRef ?? (item.kind?.toLowerCase() === "epic" ? item.ref : null),
    });
  }
  return context;
}

export function eventFilterFromUrl(
  url: URL,
  itemContext: ReadonlyMap<string, { project?: string | null; epic?: string | null }>,
) {
  return {
    kind: url.searchParams.get("kind") ?? undefined,
    refs: url.searchParams.getAll("ref"),
    actor: url.searchParams.get("actor") ?? undefined,
    project: url.searchParams.get("project") ?? undefined,
    epic: url.searchParams.get("epic") ?? undefined,
    search: url.searchParams.get("q") ?? undefined,
    itemContext,
  };
}

/** Apply a preference view verb to the views store (#1276). Returns a
 *  result note for the action record, or throws — the caller maps a
 *  throw to a rejected action so the audit trail says what happened. */
export async function applyViewAction(action: DashAction): Promise<string> {
  const owner = ownerKey(action.author);
  const pl = action.payload;
  switch (action.verb) {
    case "save-view": {
      const view = await viewsStore.save({ owner, name: pl.viewName!, filters: pl.viewFilters! });
      return `saved view ${view.id} "${view.name}"`;
    }
    case "rename-view": {
      const view = await viewsStore.rename(pl.viewId!, owner, pl.viewName!);
      if (!view) throw new Error(`no view ${pl.viewId} for this owner`);
      return `renamed view ${view.id} to "${view.name}"`;
    }
    case "delete-view": {
      const gone = await viewsStore.remove(pl.viewId!, owner);
      if (!gone) throw new Error(`no view ${pl.viewId} for this owner`);
      return `deleted view ${pl.viewId}`;
    }
    case "set-default-view": {
      if (pl.viewId === undefined) {
        await viewsStore.setDefault(null, owner);
        return "default view cleared";
      }
      const view = await viewsStore.setDefault(pl.viewId, owner);
      if (!view) throw new Error(`no view ${pl.viewId} for this owner`);
      return `default view is now ${view.id} "${view.name}"`;
    }
    default:
      throw new Error(`not a view verb: ${action.verb}`);
  }
}

/** The digest (#1278): built live from the same Source as every page;
 * the archived form is the JSON the generator script wrote. */
export async function digestContext(kind: DigestKind): Promise<Digest> {
  // Prime the items facet first (#1267 pattern): items() is the call
  // that records the pool's feed freshness into provenance — without
  // it the digest's pool sections would read an unprimed facet.
  await source.items([]).catch(() => null);
  const [pool, needsDrew, quota, poolAvailable] = await Promise.all([
    source.projects(),
    source.needsDrew ? source.needsDrew().catch(() => null) : Promise.resolve(null),
    source.quota().catch(() => null),
    source.projectsAvailable ? source.projectsAvailable().catch(() => undefined) : Promise.resolve(undefined),
  ]);
  return buildDigest({
    pool,
    needsDrew,
    quota,
    provenanceDetail: source.provenanceDetail?.() ?? null,
    poolAvailable,
    sourceKind: cfg.source,
    now: new Date(),
    kind,
  });
}

export const GRAINS = new Set<string>(["quarter", "month", "week", "3d", "today"]);

/**
 * #1267: the live source populates its provenance strings lazily, one
 * facet per method call — so a consumer that reads provenance()
 * without exercising the facets (exactly what /api/health did)
 * reports "fixtures" for facets that are really live, and its answer
 * depends on which pages were visited since boot. Health therefore
 * exercises every facet method before reading provenance. Priming is
 * cached for 60 s (the #1147 probe-cache cadence): facet reads are
 * mostly cache- or file-backed, and a watchdog polling health must
 * not fan out to every upstream on every hit. Facet failures never
 * fail health — a failed facet keeps its honest fallback string,
 * which is precisely what health exists to report.
 */
export let lastPrimedAt = 0;
/** #1141: item coverage measured by the latest priming pass (below),
 *  from the same board items() resolution that sets facet provenance. */
export let lastItemsCoverage: ItemsCoverage | null = null;
export let priming: Promise<void> | null = null;
export async function ensureProvenancePrimed(): Promise<void> {
  if (Date.now() - lastPrimedAt < 60_000) return;
  priming ??= (async () => {
    try {
      const { items } = await boardContext();
      lastItemsCoverage = summarizeItemCoverage(items);
      const refs = [...items.keys()];
      await Promise.allSettled([
        source.roadmap(),
        source.comments(refs),
        source.quota(),
        source.usage(),
        source.attention(),
        source.orchestration(),
        source.dns(),
        source.aa(),
        source.registry(),
        source.projects(),
      ]);
    } catch {
      /* facets that failed keep their fallback strings — health reports those */
    }
    lastPrimedAt = Date.now();
    priming = null;
  })();
  await priming;
}

/**
 * The one health assembly behind both /health and /api/health
 * (#1147): primed provenance first (so facet truth never depends on
 * request order, #1267), then the feed snapshot the section ages and
 * job lines are measured from, then buildHealth's probes (cached
 * 60 s inside health.ts). Page and API read this same view, so they
 * cannot disagree.
 */
export async function healthContext(prime = true): Promise<HealthView> {
  // Priming exercises every facet so /health provenance never depends
  // on request order. Callers that only need the feed snapshot and the
  // cached probes (the attention path's watchdog assembly, #1148) pass
  // prime=false: a stalled unrelated facet (e.g. Grist's roadmap API
  // and its 8s timeout) must not delay attention rendering (Codex P2
  // on PR #2431). The registry age the watchdog reads comes from the
  // live source's lastFeed, which attention's own reads already load.
  if (prime) await ensureProvenancePrimed();
  const [feed, feedFileMtimeMs, quota] = await Promise.all([
    loadFeed(cfg.feedFile).catch(() => null),
    stat(cfg.feedFile)
      .then((s) => s.mtimeMs)
      .catch(() => null),
    source.quota().catch(() => emptyQuotaView()),
  ]);
  return buildHealth({
    mode: cfg.source,
    cfg,
    feed,
    feedFileMtimeMs,
    quota,
    diagnosis: source.diagnostics?.().orchestration,
    detail: source.provenanceDetail?.(),
    itemsCoverage: lastItemsCoverage,
    comments: await commentsStore.pendingCounts().catch(() => undefined),
    conflicts: await assembleConflicts().catch(() => null),
    pushHealth: cfg.source === "fixtures" ? null : freshnessReconciler.health(new Date()),
    slo: cfg.source === "fixtures" ? null : await sloSummaries(),
    edition: cfg.source === "fixtures" ? null : await editionDrift(),
  });
}

// SLO error budgets (#9988): summaries are read from the append-only journal
// and cached for a minute, the same cadence the sampler writes at.
export const SLO_SUMMARY_CACHE_MS = 60_000;
export async function sloSummaries(nowMs = Date.now()): Promise<SloFacetSummary[]> {
  if (sloSummaryCache && nowMs - sloSummaryCache.at < SLO_SUMMARY_CACHE_MS) return sloSummaryCache.value;
  const byFacet = groupSamplesByFacet(await sloJournal.readAll().catch(() => []));
  const value = SLO_DEFINITIONS.map((definition) =>
    summarizeSloFacet(definition, byFacet.get(definition.id) ?? [], nowMs),
  );
  sloSummaryCache = { at: nowMs, value };
  return value;
}

// Edition drift (CF-9, #14949): how far this process's deployed build
// has fallen behind origin/main, measured from the build marker
// (PM_AGG_BUILD_VERSION) against the primary checkout's refs
// (PM_AGG_EDITION_REPO_DIR overrides the checkout; default ~/git/code,
// the clone pm-git-sync keeps on main). Cached five minutes: the
// inputs move at merge cadence and each measurement is a handful of
// local git calls. A failed measurement is null — the health view
// renders it as unassembled, never as an invented in-sync.
const EDITION_DRIFT_CACHE_MS = 300_000;
let editionDriftCache: { at: number; value: EditionDrift | null } | null = null;
export async function editionDrift(nowMs = Date.now()): Promise<EditionDrift | null> {
  if (editionDriftCache && nowMs - editionDriftCache.at < EDITION_DRIFT_CACHE_MS) {
    return editionDriftCache.value;
  }
  const repoDir =
    process.env.PM_AGG_EDITION_REPO_DIR?.trim() || join(requireHome("PM_AGG_EDITION_REPO_DIR"), "git", "code");
  const value = await computeEditionDrift(resolveBuildVersion(), repoDir).catch(() => null);
  editionDriftCache = { at: nowMs, value };
  return value;
}
export let sloSummaryCache: { at: number; value: SloFacetSummary[] } | null = null;
