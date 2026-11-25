// The aggregator's HTTP server. Read-mostly: pages are server-rendered
// HTML and /api/*.json serve the same data machine-first. Slice 3 (#906)
// adds the app's first writes — dashboard comments (POST /api/comments
// and its triage/github-result companions); the triage write path
// (#1152/#1154/#1156) adds dashboard actions (POST /api/actions and
// its result companion) on the same pattern. Everything underneath
// (GitHub, Kaneo, Grist) stays read-only from here — the fleet jobs
// apply actions; the dashboard only records them.
//
// Split (#13052): the wiring lives in server-wiring.ts, the context
// assemblies in server-contexts.ts, the standalone token pages in
// server-pages.ts, the Response constructors in server-respond.ts,
// and the write / alert / decision route families in
// server-write-routes.ts, server-alert-routes.ts and
// server-decision-routes.ts. This file keeps the read routes, the
// dispatch order, and the bootstrap.
import { execFile } from "node:child_process";
import { mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { Database } from "bun:sqlite";
import { buildQuotaAnalytics, primeAttemptAggregates, type QuotaAnalytics } from "./analytics-store.ts";
import { readRouteApiTarget } from "./route-rollup.ts";
import { decodePathSegment } from "./path-segment.ts";
import { errorBoundaryResponse, notFoundResponse } from "./error-boundary.ts";
import { ACTION_STATES } from "./actions.ts";
import { commentKindCounts, queuedComments } from "./comments.ts";
import { itemMatchesView } from "./views.ts";
import { filterBoardForView, filterRoadmapForView, fragBoardUrl, linkedLike } from "./board-view.ts";
import { pageEvents } from "./events.ts";
import { createFreshnessIpcHandler } from "./freshness-ipc.ts";
import { freshnessDeliveryResponse } from "./freshness-delivery.ts";
import { activityCoverageHtml, gapsHtml, ACTIVITY_SOURCE_COVERAGE } from "./activity.ts";
import { unconfiguredGithubStatus } from "./github-hot-list.ts";
import { buildOutcomeView, drillOutcomeFacts, type OutcomeDrillSelector } from "./outcome-view.ts";
import { OutcomeFactStore } from "./outcome-attribution.ts";
import { itemServiceLinks } from "./service-links.ts";
import { sectionPoolIdsFromConfig } from "./aa.ts";
import { listOpenNeedsDrew } from "./needs-drew.ts";
import { dismissDecisionSets, listOpenDecisionSets, sweepDecisionSets } from "./decisions.ts";
import { listStackHealthDown, readStackHealthStates } from "./stack-health.ts";
import { buildHealth, pageRenderHealth, recordRender } from "./health.ts";
import { resolveBwsSecret } from "./secrets.ts";
import { openMetadataStore, type EpicSlugMap, type IngestionItem } from "./metadata-ingestion.ts";
import { createMetadataWebhookHandler } from "./webhook-listener.ts";
import { requireHome } from "./home.ts";
import { createPromptWsSession, type PromptWsSession } from "./prompt-ws.ts";
import { readHostHealthSnapshot } from "./host-health.ts";
import { startPmCoreSchedulers } from "./pm-core-wiring.ts";
import { startNotifyRelay } from "./notify-relay.ts";
import { alertsIntakeAuthorized } from "./server-alert-routes.ts";
import { computeEditionDrift, type EditionDrift } from "./edition-drift.ts";
import { manifestRouteFor } from "./pages.ts";
import { allRefs, filterAttentionViewByRefs, projectPace, type RoadmapGrain } from "./derive.ts";
import { roadmapDriftReport } from "./roadmap-reconcile.ts";
import type { DashComment, ProjectPoolItem } from "./model.ts";
import { buildEpicRollups } from "./epics.ts";
import { buildProgramData } from "./program.ts";
import { buildReleaseSnapshots } from "./releases.ts";
import { deriveGenealogy, enrichRoadmapRows } from "./genealogy.ts";
import {
  PROJECT_CONFIGS,
  allProjectRefs,
  buildProjectDetail,
  buildProjectSummaries,
  projectDocTarget,
  projectConfigById,
} from "./project-config.ts";
import { PHASE12_REPO, parsePhase12Steps, type Phase12Snapshot } from "./phase12.ts";
import { PHASE_MAPPING_REPO, type PhaseMappingSnapshot } from "./phase-mapping.ts";
import type { BuildHealthSnapshot } from "./build-health.ts";
import { parseF12Items, type F12PrRef } from "./f12-readiness.ts";
import { identitiesFromRecords } from "./registry.ts";
import {
  attentionFragment,
  attentionSections,
  ghObjectRef,
  renderAa,
  renderAttention,
  renderAttentionShell,
  renderBoard,
  renderBoardBody,
  renderDiscuss,
  renderDns,
  renderConflicts,
  renderHealth,
  renderItem,
  deriveOverview,
  renderOverview,
  renderOverviewBody,
  renderOverviewShell,
  renderChat,
  renderCommentsQueue,
  renderPages,
  renderProgram,
  renderProgramProject,
  renderQuota,
  renderQueue,
  renderQueueBody,
  renderQueueShell,
  renderRoadmap,
  renderSearch,
  renderServiceDetail,
  renderSessions,
  renderToday,
  renderTodayBody,
  renderTodayShell,
  eventTableHtml,
} from "./render.ts";
import { buildBriefs } from "./brief.ts";
import { listDigestArchives, readDigestArchive } from "./digest.ts";
import { classEnabled, getAlertsStore, startAlertEngine, type PushPayload } from "./alerts.ts";
import { buildIncidentEnvelope, incidentPageThreshold, incidentTarget, renderIncidentPage } from "./incident-page.ts";
import { recordSwProbe, sendPush, swProbeSummary } from "./push.ts";
import { renderDigest, esc } from "./render.ts";
import type { QueueSnapshot } from "./queue.ts";
import { deriveSoleDoorPoolIds, type NotificationSnapshot } from "./notifications.ts";
import { readJsonBody } from "./request-body.ts";
import { parseRegistryQuery, queryRegistry } from "./registry-view.ts";
import { addConsoleShortcut } from "./console-gate.ts";
import type { Source } from "./source.ts";
import { boardJsonBody, boardJsonViewBody } from "./board-json.ts";
import { sampleSloFacets } from "./slo-sampler.ts";
import type { EscalationSink } from "./escalation.ts";
import { createEscalationTicker, escalationDelivery, escalationJournalLine } from "./escalation-tick.ts";
import { emptyState } from "./remediation.ts";
import { computeFlowAnalytics } from "./flow-analytics.ts";
import { FORECAST_CONTRACT_VERSION } from "./forecast-contract.ts";
import {
  cfg,
  dashboardResponseWithinDeadline,
  roadmapAnnotationsStore,
  itemHistoryStore,
  sloJournal,
  promptsStore,
  loadSpendCards,
  readyAgentDispatchStates,
  coverageSnapshot,
  actionsStore,
  commentsStore,
  viewsStore,
  eventsStore,
  freshnessStore,
  freshnessReconciler,
  activityStore,
  githubHotListConsumer,
  githubOutcomeConsumer,
  githubActivityStatus,
  activitySourceStatusHtml,
  recordActivity,
  source,
  quotaRefresher,
  briefCommentsFor,
  assembleConflicts,
  commentIdentities,
  commentFormOpts,
  discussTitle,
  emptyQuotaView,
  emptyUsageView,
} from "./server-wiring.ts";
import { json, jsonCompact, html, serviceWorkerResponse } from "./server-respond.ts";
import { questionsHtml, PWA_MANIFEST } from "./server-pages.ts";
import {
  boardContext,
  allForecastContracts,
  roadmapContext,
  searchContext,
  deriveWatchdogItems,
  attentionContext,
  ssePatch,
  sseResponse,
  boardRender,
  attentionRender,
  todaySignals,
  todayContext,
  queueContext,
  overviewContext,
  programSources,
  resolveView,
  poolItemLike,
  eventContextByRef,
  eventFilterFromUrl,
  digestContext,
  GRAINS,
  healthContext,
  sloSummaries,
  withSearchTiming,
  withSearchTimings,
} from "./server-contexts.ts";
import {
  handleReportsRead,
  handleReportWriteRoutes,
  handleIdentityRoutes,
  handleWriteRoutes,
} from "./server-write-routes.ts";
import {
  handleAlertIntakeRoutes,
  handleAlertAction,
  handleAlertRead,
  handleAlertsList,
  handleAlertPreferences,
  handleNotificationsPage,
  UUID_RE,
} from "./server-alert-routes.ts";
import { handleDecisionRoutes, handleDecisionsIndex, handleDecisionTailRoutes } from "./server-decision-routes.ts";

function runMetadataFleet(args: string[], input?: string): Promise<string> {
  const fleetBin = process.env.FLEET_BIN ?? "fleet";
  return new Promise((resolve, reject) => {
    const child = execFile(fleetBin, args, { maxBuffer: 64 * 1024 * 1024, timeout: 60_000 }, (error, stdout) =>
      error ? reject(new Error(`fleet ${args[0]} failed: ${(error as Error).message}`)) : resolve(stdout),
    );
    if (input !== undefined && child.stdin) {
      child.stdin.write(input);
      child.stdin.end();
    }
  });
}

async function withMetadataTempFile<T>(prefix: string, content: string, run: (path: string) => Promise<T>): Promise<T> {
  const directory = await mkdtemp(join(tmpdir(), prefix));
  const path = join(directory, "payload.json");
  try {
    await writeFile(path, content, { mode: 0o600 });
    return await run(path);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
}

const metadataDbPath =
  process.env.PM_METADATA_DB_PATH ??
  join(requireHome("PM_METADATA_DB_PATH"), ".local/state/pm-aggregator/pm-metadata.sqlite");
const metadataDb = openMetadataStore(metadataDbPath, (path) => new Database(path));
const metadataWebhookHandler = createMetadataWebhookHandler({
  secret: () => resolveBwsSecret("pm-aggregator.github-webhook-secret"),
  db: metadataDb,
  epicSlugs: async (item: IngestionItem): Promise<EpicSlugMap> => {
    const output = await runMetadataFleet([
      "gh",
      "issue",
      "list",
      "--repo",
      item.repo,
      "--state",
      "all",
      "--limit",
      "1000",
      "--json",
      "number,labels",
    ]);
    const rows: unknown = JSON.parse(output);
    if (!Array.isArray(rows)) throw new Error(`unexpected epic list shape for ${item.repo}`);
    const slugs: EpicSlugMap = new Map();
    for (const row of rows as { number?: unknown; labels?: ({ name?: unknown } | string)[] }[]) {
      if (!Number.isSafeInteger(row.number) || (row.number as number) < 1) continue;
      const labels = (row.labels ?? []).map((label) => (typeof label === "string" ? label : String(label.name ?? "")));
      if (!labels.includes("kind/epic")) continue;
      for (const label of labels) {
        const slug = /^epic\/(.+)$/.exec(label)?.[1]?.trim().toLowerCase();
        if (slug && !slugs.has(slug)) slugs.set(slug, { repo: item.repo, number: row.number as number });
      }
    }
    return slugs;
  },
  writeBack: async (item, body) => {
    await withMetadataTempFile("pm-metadata-webhook-", body, async (path) => {
      await runMetadataFleet([
        "gh",
        item.kind === "issue" ? "issue" : "pr",
        "edit",
        String(item.number),
        "--repo",
        item.repo,
        "--body-file",
        path,
      ]);
    });
  },
  enqueueTriage: async (packet) => {
    await withMetadataTempFile("pm-metadata-triage-", JSON.stringify(packet, null, 2), async (path) => {
      await runMetadataFleet(["queue", "enqueue", path, "--queue", "repo-health-code-pilot"]);
    });
  },
});

async function handler(req: Request, remoteAddress?: string): Promise<Response> {
  const url = new URL(req.url);
  const path = url.pathname;
  const prov = () => source.provenance();
  const provDetail = () => source.provenanceDetail?.();

  if (path === "/webhooks/github") return metadataWebhookHandler(req);

  if (req.method === "GET" && (path === "/reports" || path === "/api/reports.json")) {
    return handleReportsRead(req, path);
  }

  // C6 incident page (#15019, design §6): one evidence-backed page
  // per threshold-meeting incident, assembled live from the alerts
  // ledger's healthHook records. The path id is the alerts-ledger
  // incident id; the page identity joins on healthHook.incidentId.
  const incidentPageMatch = /^\/reports\/incident\/([\w-]+)$/.exec(path);
  const incidentJsonMatch = /^\/api\/reports\/incident\/([\w-]+)\.json$/.exec(path);
  if (req.method === "GET" && (incidentPageMatch || incidentJsonMatch)) {
    const id = (incidentPageMatch ?? incidentJsonMatch)![1];
    if (!UUID_RE.test(id)) return json({ error: "incident id must be a uuid" }, 400);
    const store = await getAlertsStore();
    if (!store) return json({ error: "alerts store unavailable" }, 503);
    const incident = await store.getIncident(id);
    if (!incident) return json({ error: "no such incident", id }, 404);
    const events = await store.listIncidentEvents(id);
    if (!events.some((e) => e.detail?.healthHook !== undefined)) {
      return json({ error: "incident has no healthHook evidence records", id }, 404);
    }
    const target = incidentTarget(incident);
    const related = (await store.listIncidents(200)).filter((r) => target !== null && incidentTarget(r) === target);
    if (!incidentPageThreshold(incident, events, related).qualifies) {
      return json({ error: "incident does not meet the C6 section 6.2 page threshold", id }, 404);
    }
    const envelope = buildIncidentEnvelope({
      incident,
      events,
      related,
      generatedAt: new Date().toISOString(),
    });
    return incidentJsonMatch ? json(envelope) : html(renderIncidentPage(envelope));
  }

  const reportWriteResponse = await handleReportWriteRoutes(req, path);
  if (reportWriteResponse) return reportWriteResponse;

  // --- #3978: freshness delivery for browsers ------------------------------
  // GET /freshness/stream (SSE) and GET /api/freshness-events.json read the
  // #3977 journal only. Scope is server-derived from the page name; a caller
  // cannot widen it, and no notice carries source payload or a secret.
  if (req.method === "GET" && (path === "/freshness/stream" || path === "/api/freshness-events.json")) {
    const delivery = await freshnessDeliveryResponse(req, url, { reader: freshnessStore });
    if (delivery !== null) return delivery;
  }

  // --- Author identities for the on-demand picker (#1179) ------------------
  // List pages render author pickers with the standing identities only;
  // the full standing + per-session list is fetched once, on first use.
  if (req.method === "GET" && path === "/api/author-identities") {
    const records = await source.registry().catch(() => []);
    return json({ identities: identitiesFromRecords(records) });
  }

  // #9604/#9605: identity-registry admin + stack-token exchange — server-write-routes.ts (#13052).
  const identityResponse = await handleIdentityRoutes(req, path);
  if (identityResponse) return identityResponse;

  // Dashboard writes (#906 §2, #1152/#1154/#1156, #1276/#1277): prompt, comment and
  // action route families — server-write-routes.ts (#13052).
  const writeResponse = await handleWriteRoutes(req, url, path, remoteAddress);
  if (writeResponse) return writeResponse;

  if (path === "/api/events.json") {
    // #1739/#3323: the same source-native stream and filters as /events.
    const pool = await source.projects().catch(() => [] as ProjectPoolItem[]);
    const filtered = await eventsStore.list(eventFilterFromUrl(url, eventContextByRef(pool)));
    const page = pageEvents(filtered, {
      cursor: url.searchParams.get("cursor"),
      limit: Number(url.searchParams.get("limit") ?? 50),
    });
    // #3322 slice A: the audit health surface rides along — pending
    // retry-outbox count + hash-chain integrity verdict.
    const [health, checkpoints, gaps] = await Promise.all([
      eventsStore.health(),
      activityStore.checkpoints(),
      activityStore.gaps(),
    ]);
    return json({
      events: page.events,
      health,
      pagination: { total: filtered.length, snapshotAt: page.snapshotAt, nextCursor: page.nextCursor },
      activity: {
        checkpoints,
        gaps,
        coverage: ACTIVITY_SOURCE_COVERAGE,
        freshness: {
          status: "unknown",
          reason: "ingestion checkpoints record deliveries, not successful source polls",
        },
      },
    });
  }
  if (path === "/events") {
    // Machine and human surfaces share filters, ordering, and a stable
    // ingestion snapshot for cursor pagination.
    const pool = await source.projects().catch(() => [] as ProjectPoolItem[]);
    const filtered = await eventsStore.list(eventFilterFromUrl(url, eventContextByRef(pool)));
    const page = pageEvents(filtered, {
      cursor: url.searchParams.get("cursor"),
      limit: Number(url.searchParams.get("limit") ?? 50),
    });
    const projects = [...new Set(pool.map((item) => item.projectUrl))].sort();
    const epics = new Map<string, string>();
    for (const item of pool) {
      if (item.kind?.toLowerCase() === "epic") epics.set(item.ref, item.title);
      if (item.epicRef)
        epics.set(item.epicRef, pool.find((candidate) => candidate.ref === item.epicRef)?.title ?? item.epicRef);
    }
    const refs = url.searchParams.getAll("ref");
    const option = (value: string, label: string, selected: string) =>
      `<option value="${esc(value)}"${value === selected ? " selected" : ""}>${esc(label)}</option>`;
    const projectOptions = [...new Set([url.searchParams.get("project") ?? "", ...projects].filter(Boolean))]
      .map((value) => option(value, value, url.searchParams.get("project") ?? ""))
      .join("");
    const selectedEpic = url.searchParams.get("epic") ?? "";
    if (selectedEpic && !epics.has(selectedEpic)) epics.set(selectedEpic, selectedEpic);
    const epicOptions = [...epics.entries()]
      .map(([value, label]) => option(value, label, url.searchParams.get("epic") ?? ""))
      .join("");
    // Every ref filter is a visible, editable input: hidden preserved
    // refs could not be removed, so clearing the visible Ref field
    // still restricted results to the item's linked refs (Codex P2,
    // PR #5266). A cleared input submits ref="", which filterEvents
    // ignores, so clearing an input is removing that ref.
    const refInputs = (refs.length ? refs : [""])
      .map((ref, index) => `<label>Ref${index ? ` ${index + 1}` : ""} <input name="ref" value="${esc(ref)}"></label>`)
      .join("");
    const nextUrl = page.nextCursor ? new URL(url) : null;
    if (nextUrl && page.nextCursor) nextUrl.searchParams.set("cursor", page.nextCursor);
    const apiUrl = new URL(url);
    apiUrl.pathname = "/api/events.json";
    apiUrl.hash = "";
    return html(
      `<!doctype html><html><head><meta charset="utf-8"><title>Events — pm-stack</title>` +
        `<style>body{margin:1rem;font:16px system-ui,sans-serif}form{display:flex;flex-wrap:wrap;align-items:end;gap:.75rem}form label{display:grid;gap:.25rem}.tblwrap{max-width:100%;overflow-x:auto;-webkit-overflow-scrolling:touch}table{border-collapse:collapse;min-width:36rem}th,td{border:1px solid #888;padding:.4rem .55rem;text-align:left;vertical-align:top}.muted{color:#666}</style></head><body>` +
        `<h1>Events</h1><p>Showing ${page.events.length} of ${filtered.length} matching events from a fixed snapshot, newest first. ` +
        `Machine twin: <a href="${esc(apiUrl.pathname + apiUrl.search)}">/api/events.json</a>.</p>` +
        `<p class="muted"><strong>Freshness: unknown.</strong> Ingestion checkpoints record deliveries, not successful source polls.</p>` +
        `<form method="get" action="/events" aria-label="Filter activity events">` +
        `<label>Project <select name="project"><option value="">All projects</option>${projectOptions}</select></label>` +
        `<label>Epic <select name="epic"><option value="">All epics</option>${epicOptions}</select></label>` +
        `<label>Kind <input name="kind" value="${esc(url.searchParams.get("kind") ?? "")}"></label>` +
        `<label>Actor <input name="actor" value="${esc(url.searchParams.get("actor") ?? "")}"></label>` +
        refInputs +
        `<label>Search <input name="q" value="${esc(url.searchParams.get("q") ?? "")}"></label>` +
        `<label>Page size <select name="limit">${[25, 50, 100].map((n) => option(String(n), String(n), url.searchParams.get("limit") ?? "50")).join("")}</select></label>` +
        `<button type="submit">Filter</button></form>` +
        eventTableHtml(page.events) +
        (nextUrl ? `<p><a rel="next" href="${esc(nextUrl.pathname + nextUrl.search)}">Next page →</a></p>` : "") +
        `<h2>Activity source status</h2>${activitySourceStatusHtml(await githubActivityStatus())}` +
        `<h2>Activity gaps</h2><div class="tblwrap">` +
        gapsHtml(await activityStore.gaps()) +
        `</div><h2>Activity source coverage</h2><div class="tblwrap">` +
        activityCoverageHtml() +
        `</div>` +
        `</body></html>`,
    );
  }
  if (path === "/api/activity.json") {
    // #3321: activity-ingestion state — per-source checkpoints, gap
    // records, and the documented source coverage. The machine twin
    // of the sections the /events page renders below its table.
    const [checkpoints, gaps, sources] = await Promise.all([
      activityStore.checkpoints(),
      activityStore.gaps(),
      activityStore.sourceStatuses(),
    ]);
    const github = sources.find((status) => status.source === "github") ?? unconfiguredGithubStatus();
    return json({
      checkpoints,
      gaps,
      sources: [{ ...github, completeHistory: false }],
      coverage: ACTIVITY_SOURCE_COVERAGE,
    });
  }
  if (path === "/api/views.json") {
    // #1276: the saved views, per owner (?owner=<kind>:<id>) or all.
    const owner = url.searchParams.get("owner");
    const views = owner ? await viewsStore.forOwner(owner) : await viewsStore.all();
    return json({ views });
  }
  if (path === "/api/actions.json") {
    const object = url.searchParams.get("object");
    const kind = url.searchParams.get("kind");
    const state = url.searchParams.get("state");
    // #4620: state=all is the explicit all-history read (audit and
    // retrospective consumers). It must not fall through to the literal
    // state filter, which silently returned an empty list while the
    // store held applied/rejected history — a false zero. Any other
    // value that is not a real action state is a 400, never an empty
    // list, so a mistyped filter cannot masquerade as "no history".
    if (state && state !== "all" && !ACTION_STATES.has(state)) {
      return json({ error: `state must be one of: all, ${[...ACTION_STATES].join(", ")}` }, 400);
    }
    let list = await actionsStore.all();
    if (object) list = list.filter((a) => a.objectRef === object);
    if (kind) list = list.filter((a) => a.kind === kind);
    if (state && state !== "all") list = list.filter((a) => a.state === state);
    return json({ actions: list, pending: await actionsStore.pendingCounts() });
  }

  if (path === "/api/comments.json") {
    const object = url.searchParams.get("object");
    const kind = url.searchParams.get("kind");
    const state = url.searchParams.get("state");
    let list: DashComment[] = await commentsStore.all();
    if (object) list = list.filter((c) => c.objectRef === object);
    if (kind) list = list.filter((c) => c.kind === kind);
    if (state === "new") list = list.filter((c) => c.triageState === "new" || c.githubState === "pending");
    const all = await commentsStore.all();
    return json({
      comments: list,
      pending: {
        messages: all.filter((c) => c.kind === "message" && c.triageState === "new").length,
        github: all.filter((c) => c.kind === "github" && c.githubState === "pending").length,
      },
    });
  }

  if (path === "/") {
    // #1134: the front door is the overview — shell first, the
    // summary streams in as a fragment (#1444 pattern). ?full=1
    // keeps the full server render (no-JS path).
    if (url.searchParams.get("full") !== "1") {
      return html(renderOverviewShell(prov(), provDetail()));
    }
    return html(renderOverview(await overviewContext(), prov(), provDetail()));
  }
  if (path === "/frag/overview") {
    const data = await overviewContext();
    const wrapped = `<div id="overview-main" data-frag data-on:dashrefresh__window="@get('/frag/overview')">${renderOverviewBody(deriveOverview(data))}</div>`;
    return sseResponse(ssePatch("#overview-main", wrapped));
  }
  if (path === "/board") {
    const { board, items, counts, act } = await boardRender();
    const resolved = await resolveView(url);
    if (!resolved) {
      return html(
        renderBoard(board, items, prov(), counts, provDetail(), act, {
          showAllDone: url.searchParams.get("done") === "all",
        }),
      );
    }
    const { board: keptBoard, items: keptItems } = filterBoardForView(board, items, act.poolByRef, resolved.filters);
    return html(
      renderBoard(keptBoard, keptItems, prov(), counts, provDetail(), act, {
        showAllDone: url.searchParams.get("done") === "all",
      }),
    );
  }
  if (path === "/frag/board") {
    const { board, items, counts, act } = await boardRender();
    // #8521: the fragment twin applies the same saved view as /board
    // (PR #4013 missed it), and its refresh URLs re-name the view so
    // a reload does not silently drop the filter. #7937: the same
    // treatment for ?done=all — the fragment renders the full Done
    // strip when the flag is present (PR #3855 wired it only into
    // /board), and fragBoardUrl carries it across refreshes.
    const fragUrl = fragBoardUrl(url.searchParams);
    const resolved = await resolveView(url);
    const view = resolved ? filterBoardForView(board, items, act.poolByRef, resolved.filters) : { board, items };
    const wrapped = `<div id="board-main" data-frag data-on:dashrefresh__window="@get('${fragUrl}')">${renderBoardBody(view.board, view.items, counts, act, { showAllDone: url.searchParams.get("done") === "all" })}</div>`;
    return sseResponse(ssePatch("#board-main", wrapped));
  }

  if (path === "/today") {
    if (url.searchParams.get("full") !== "1") {
      const signals = await todaySignals();
      // #9354: the Questions section lives in the SHELL, outside the
      // #today-main fragment div (see the /frag/today note below).
      return html(
        renderTodayShell(
          prov(),
          provDetail(),
          signals.health,
          signals.quota,
          signals.quotaUnavailable,
          await questionsHtml(),
        ),
      );
    }
    const { today, view, coverage, quota, health, quotaUnavailable, queue, queueStale } = await todayContext();
    return html(
      renderToday(
        today,
        view,
        quota,
        coverage,
        prov(),
        provDetail(),
        health,
        quotaUnavailable,
        queue,
        queueStale,
        await questionsHtml(),
      ),
    );
  }
  if (path === "/frag/today") {
    // #9354: questionsHtml is deliberately NOT passed here — the
    // /today shell already renders the Questions section outside the
    // #today-main fragment div, so a fragment refresh carrying it too
    // would show the section twice on the live page. The shell's copy
    // is from page load, which its footer states. (?full=1 never uses
    // the shell, so it passes questionsHtml via renderToday instead.)
    const { today, view, coverage, quota, health, quotaUnavailable, queue, queueStale } = await todayContext();
    const wrapped = `<div id="today-main" data-frag data-on:dashrefresh__window="@get('/frag/today')">${renderTodayBody(today, view, quota, coverage, health, quotaUnavailable, queue, queueStale)}</div>`;
    return sseResponse(ssePatch("#today-main", wrapped));
  }

  if (path === "/digest") {
    const kind = url.searchParams.get("kind") === "weekly" ? "weekly" : "daily";
    const date = url.searchParams.get("date");
    const archives = await listDigestArchives(cfg.digestsDir);
    // The dated archive URL (#1278): renders the stored digest verbatim.
    if (date && /^\d{4}-\d{2}-\d{2}$/.test(date)) {
      const archived =
        (await readDigestArchive(cfg.digestsDir, date, kind)) ??
        (kind === "daily" ? null : await readDigestArchive(cfg.digestsDir, date, "daily"));
      if (archived) return html(renderDigest(archived, archives, prov(), provDetail()));
      return html(renderDigest(await digestContext("daily"), archives, prov(), provDetail()), 404);
    }
    return html(renderDigest(await digestContext(kind), archives, prov(), provDetail()));
  }
  if (path === "/queue") {
    if (url.searchParams.get("full") !== "1") return html(renderQueueShell(prov(), provDetail()));
    return html(renderQueue(await queueContext(), prov(), provDetail()));
  }
  if (path === "/frag/queue") {
    const view = await queueContext();
    const wrapped = `<div id="queue-main" data-frag data-on:dashrefresh__window="@get('/frag/queue')">${renderQueueBody(view)}</div>`;
    return sseResponse(ssePatch("#queue-main", wrapped));
  }

  if (path === "/roadmap") {
    const { rows, items, pool, reconciliation, drill } = await roadmapContext();
    const orchestration = await source.orchestration().catch(() => null);
    const orchestrationStatus = source.diagnostics?.().orchestration.state;
    const poolRefs = new Set(pool.map((item) => item.ref));
    const briefRefs = [...new Set(rows.flatMap((row) => allRefs([row]).filter((ref) => poolRefs.has(ref))))];
    const briefs = buildBriefs(pool, {
      comments: briefRefs.length ? await briefCommentsFor(briefRefs) : [],
      orchestration,
      orchestrationStatus,
    });
    const forecasts = (await allForecastContracts(pool)).byRef;
    // #4830: the same twin /api/flow-analytics.json serves, computed
    // the same way — the Roadmap charts render exactly this object.
    const flowAnalytics = computeFlowAnalytics(await itemHistoryStore.readAll().catch(() => []));
    const spendCards = await loadSpendCards();
    const releaseSnapshots = buildReleaseSnapshots(pool);
    const prioritiesView = url.searchParams.get("view") === "priorities";
    const g = url.searchParams.get("h") ?? "quarter";
    const grain: RoadmapGrain = GRAINS.has(g) ? (g as RoadmapGrain) : "quarter";
    const resolved = await resolveView(url);
    // Codex P2, PR #4013: grain links inside renderRoadmap must keep
    // the active view parameters, or switching grain drops the view.
    const viewParams = new URLSearchParams();
    for (const [k, v] of url.searchParams) {
      if (k === "view" || k === "viewer" || k.startsWith("vf.")) viewParams.set(k, v);
    }
    const viewQuery = viewParams.toString();
    const annotationsFor = async (rs: typeof rows) =>
      new Map(
        await Promise.all(
          rs.map(async (row) => [row.subject, await roadmapAnnotationsStore.forObject(row.subject)] as const),
        ),
      );
    if (!resolved) {
      return html(
        renderRoadmap(
          rows,
          items,
          prov(),
          grain,
          await commentsStore.countsDetailed(),
          provDetail(),
          reconciliation,
          drill,
          await annotationsFor(rows),
          viewQuery,
          briefs,
          forecasts,
          flowAnalytics,
          spendCards,
          releaseSnapshots,
          prioritiesView,
        ),
      );
    }
    const poolByRef = new Map(pool.map((p) => [p.ref, p]));
    // Codex P2, PR #4013: items linked from a KEPT row keep their
    // metadata (links/badges in roadmapCard) even when they fail the
    // item filter themselves — same rule the board handler applies.
    // Shared with /api/roadmap.json via filterRoadmapForView (#8723).
    const { rows: keptRows, items: keptItems } = filterRoadmapForView(rows, items, poolByRef, resolved.filters);
    return html(
      renderRoadmap(
        keptRows,
        keptItems,
        prov(),
        grain,
        await commentsStore.countsDetailed(),
        provDetail(),
        reconciliation,
        drill,
        await annotationsFor(keptRows),
        viewQuery,
        briefs,
        forecasts,
        flowAnalytics,
        spendCards,
        releaseSnapshots,
        prioritiesView,
      ),
    );
  }

  if (path === "/api/flow-analytics.json") {
    // #4829 (step 2 of #1281): the flow-analytics JSON twin — the
    // exact numbers the step-3 charts (#4830) will render, computed
    // from the #3981 item-history store. Every group carries its
    // computation window and gap signal via the shared window block.
    const cutoffs = await itemHistoryStore.readAll().catch(() => []);
    return json({
      generatedAt: new Date().toISOString(),
      ...computeFlowAnalytics(cutoffs),
    });
  }

  if (path === "/api/outcomes.json") {
    // #10910: the outcome/rollup view over the append-only outcome-fact
    // ledger (outcome-view.ts) — totals plus per-job/model/session
    // groups, with drill-through (?drill=job&key=..., ?drill=session&key=...,
    // ?drill=model&provider=...&model=..., ?drill=issue&ref=...,
    // ?drill=unattributed&reason=...) returning the exact facts behind
    // an aggregate, each with its canonical source URL. A corrupt
    // ledger is an explicit error, never an empty view: a gap in the
    // evidence must not render as zero activity.
    let outcomeFacts: Awaited<ReturnType<OutcomeFactStore["readFacts"]>>;
    try {
      outcomeFacts = await new OutcomeFactStore(cfg.outcomeFile).readFacts();
    } catch {
      return json({ error: "outcome_fact_store_corrupt" }, 500);
    }
    const drill = url.searchParams.get("drill");
    if (drill === null) {
      return json({ generatedAt: new Date().toISOString(), ...buildOutcomeView(outcomeFacts) });
    }
    let selector: OutcomeDrillSelector | null = null;
    if (drill === "job" || drill === "session") {
      const key = url.searchParams.get("key")?.trim();
      if (key) selector = { dimension: drill, key };
    } else if (drill === "model") {
      selector = {
        dimension: "model",
        providerID: url.searchParams.get("provider")?.trim() || null,
        modelID: url.searchParams.get("model")?.trim() || null,
      };
    } else if (drill === "issue") {
      const ref = url.searchParams.get("ref")?.trim();
      if (ref) selector = { dimension: "issue", subjectRef: ref };
    } else if (drill === "unattributed") {
      const reason = url.searchParams.get("reason")?.trim();
      selector = {
        dimension: "unattributed",
        reason: reason === "no-binding" || reason === "source-gap" ? reason : null,
      };
    }
    if (!selector) return json({ error: "invalid_outcome_drill" }, 400);
    return json({
      generatedAt: new Date().toISOString(),
      selector,
      facts: drillOutcomeFacts(outcomeFacts, selector),
    });
  }

  if (path === "/api/forecasts.json") {
    // #3984: the shared forecast contract — the identical objects the
    // Roadmap and Projects pages render as one-line summaries.
    const pool = await source.projects().catch(() => []);
    const { contracts } = await allForecastContracts(pool);
    const subject = url.searchParams.get("subject");
    return json({
      contractVersion: FORECAST_CONTRACT_VERSION,
      generatedAt: new Date().toISOString(),
      contracts: subject ? contracts.filter((c) => c.subjectRef === subject) : contracts,
    });
  }

  if (path === "/api/quota-analytics.json") {
    // The analytics & projection model behind /quota's section —
    // the identical object the page renders from.
    const [quota, usage] = await Promise.all([
      source.quota().catch(() => emptyQuotaView()),
      source.usage().catch(() => emptyUsageView()),
    ]);
    try {
      return json(
        buildQuotaAnalytics({
          quota,
          usage,
          quotaCurrentPath: cfg.quotaCurrentFile,
          routeApi: readRouteApiTarget(cfg.opencodeServerStateFile),
          mcpxCallLedgerDir: cfg.mcpxCallLedgerDir,
          fleetInvocationLedgerFile: cfg.fleetInvocationLedgerFile,
        }),
      );
    } catch {
      return json({ error: "analytics unavailable" }, 503);
    }
  }

  if (path === "/quota") {
    const [quota, usage] = await Promise.all([
      source.quota().catch(() => emptyQuotaView()),
      source.usage().catch(() => emptyUsageView()),
    ]);
    // Never block on providers (#1171): decide from the stored measurement,
    // ask the gatherer to run in the background, render what we have.
    const refresh = quotaRefresher.request(quota);
    // Analytics fail-soft: a broken ledger read must never take the
    // quota page down with it (the builder itself also degrades).
    let analytics: QuotaAnalytics | null = null;
    try {
      analytics = buildQuotaAnalytics({
        quota,
        usage,
        quotaCurrentPath: cfg.quotaCurrentFile,
        routeApi: readRouteApiTarget(cfg.opencodeServerStateFile),
        mcpxCallLedgerDir: cfg.mcpxCallLedgerDir,
        fleetInvocationLedgerFile: cfg.fleetInvocationLedgerFile,
      });
    } catch {
      analytics = null;
    }
    return html(renderQuota(quota, prov(), usage, provDetail(), refresh, analytics));
  }

  if (req.method === "POST" && path === "/quota/refresh") {
    const quota = await source.quota().catch(() => emptyQuotaView());
    quotaRefresher.manual(quota);
    return new Response(null, { status: 303, headers: { Location: "/quota" } });
  }

  const serviceMatch = /^\/quota\/service\/([\w-]+)$/.exec(path);
  if (serviceMatch) {
    const usage = await source.usage().catch(() => emptyUsageView());
    const svc = usage.services.find((s) => s.id === serviceMatch[1]);
    if (svc) return html(renderServiceDetail(svc, usage.history[svc.id] ?? [], prov(), provDetail()));
    return html(
      `<h1>Not found</h1><p class="muted">no SaaS service with id <code>${esc(serviceMatch[1])}</code> — <a href="/quota">back to Usage &amp; billing</a></p>`,
      404,
    );
  }

  if (path === "/attention") {
    // #1444: shell first; header/queues/sessions stream in as
    // fragments. ?full=1 keeps the full server render (no-JS path).
    // ?refs=<objectRefs> is the digest landing filter (#7193 Codex):
    // only the named entries render, and the refs ride the fragment
    // URLs so the streamed sections stay filtered.
    const refsParam = url.searchParams.get("refs");
    const refs = refsParam
      ? refsParam
          .split(",")
          .map((ref) => ref.trim())
          .filter(Boolean)
      : [];
    if (url.searchParams.get("full") !== "1") {
      // #9354: Questions section in the shell, below the sessions
      // fragment; the /frag/attention/* fragments do not carry it.
      return html(renderAttentionShell(prov(), provDetail(), refsParam ?? undefined, await questionsHtml()));
    }
    const rendered = await attentionRender();
    const view = refs.length ? filterAttentionViewByRefs(rendered.view, refs) : rendered.view;
    return html(
      renderAttention(
        view,
        rendered.coverage,
        prov(),
        rendered.counts,
        provDetail(),
        rendered.act,
        rendered.briefs,
        rendered.census,
        await questionsHtml(),
      ),
    );
  }
  const attnFragMatch = /^\/frag\/attention\/(header|queues|sessions)$/.exec(path);
  if (attnFragMatch) {
    const section = attnFragMatch[1] as "header" | "queues" | "sessions";
    const refsParam = url.searchParams.get("refs");
    const refs = refsParam
      ? refsParam
          .split(",")
          .map((ref) => ref.trim())
          .filter(Boolean)
      : [];
    const rendered = await attentionRender();
    const view = refs.length ? filterAttentionViewByRefs(rendered.view, refs) : rendered.view;
    const sections = attentionSections(
      view,
      rendered.coverage,
      rendered.counts,
      rendered.act,
      rendered.briefs,
      rendered.census,
    );
    return sseResponse(
      ssePatch(`#attn-${section}`, attentionFragment(section, sections[section], refsParam ?? undefined)),
    );
  }
  if (path === "/assets/app.css") {
    // The committed Tailwind + DaisyUI build (Phase C): source is
    // src/ui.css, rebuilt with `bun run build:css`; never generated at
    // request time, so a plain pull deploys it like the other assets.
    const file = Bun.file(new URL("./assets/app.css", import.meta.url));
    if (!(await file.exists())) return json({ error: "asset missing" }, 404);
    return new Response(file, {
      headers: { "content-type": "text/css; charset=utf-8", "cache-control": "public, max-age=3600" },
    });
  }
  if (path === "/assets/datastar.js") {
    // The vendored Datastar bundle (#1444) — a static asset, never a
    // service worker; /sw.js keeps serving the kill switch.
    const file = Bun.file(new URL("./assets/datastar.js", import.meta.url));
    if (!(await file.exists())) return json({ error: "asset missing" }, 404);
    return new Response(file, {
      headers: { "content-type": "text/javascript; charset=utf-8", "cache-control": "public, max-age=3600" },
    });
  }
  if (path === "/assets/aa-explorer-computations.js") {
    // Shared with Bun tests so the browser and regression suite exercise the
    // same Cost Explorer calculations.
    const file = Bun.file(new URL("./assets/aa-explorer-computations.js", import.meta.url));
    if (!(await file.exists())) return json({ error: "asset missing" }, 404);
    return new Response(file, {
      headers: { "content-type": "text/javascript; charset=utf-8", "cache-control": "public, max-age=3600" },
    });
  }

  if (path === "/dns") {
    return html(renderDns(await source.dns(), prov(), provDetail()));
  }
  if (path === "/api/dns.json") {
    const section = await source.dns();
    return json(section ?? { status: "absent", reason: "no DNS register section in the fleet feed yet" });
  }

  if (path === "/aa") {
    return html(renderAa(await source.aa(url.searchParams.get("batch")), prov(), provDetail()));
  }
  if (path === "/api/aa.json") {
    const section = await source.aa(url.searchParams.get("batch"));
    return json(section ?? { status: "absent", reason: "no AA explorer section in the fleet feed yet" });
  }

  if (path === "/sessions") {
    const records = await source.registry();
    // #1209: window/filter/pagination, shared with the JSON twin.
    const view = queryRegistry(records, parseRegistryQuery(url.searchParams));
    return html(
      renderSessions(
        view.records,
        prov(),
        provDetail(),
        {
          actionsByObject: await actionsStore.byObject(),
          identities: identitiesFromRecords(records),
        },
        view,
      ),
    );
  }
  if (path === "/search") {
    const started = performance.now();
    const { query, results, items, timings } = await searchContext(url.searchParams.get("q") ?? "");
    const body = await withSearchTiming(timings, "render", () =>
      renderSearch(query, results, items, prov(), provDetail()),
    );
    timings.total = performance.now() - started;
    return withSearchTimings(html(body), timings);
  }
  if (path === "/api/search.json") {
    const started = performance.now();
    const { query, results, timings } = await searchContext(url.searchParams.get("q") ?? "");
    const response = await withSearchTiming(timings, "serialize", () =>
      json({ query: query.trim(), total: results.length, results }),
    );
    timings.total = performance.now() - started;
    return withSearchTimings(response, timings);
  }
  if (path === "/api/registry.json") {
    const records = await source.registry();
    // #1209: same query pipeline as /sessions — same params, same page.
    const view = queryRegistry(records, parseRegistryQuery(url.searchParams));
    return json({
      records: view.records,
      identities: identitiesFromRecords(records),
      total: view.matched,
      totalAll: view.totalAll,
      hiddenByWindow: view.hiddenByWindow,
      hiddenByPagination: view.hiddenByPagination,
      remainingAfterPage: view.remainingAfterPage,
      windowDays: view.query.windowDays,
      limit: view.query.limit,
      offset: view.query.offset,
    });
  }

  if (path === "/projects") {
    // #1497: Projects merged into Program status — the pool view is
    // the #projects-pool section of /program now. The route redirects
    // (area filter preserved); /api/projects.json below is unchanged.
    const area = url.searchParams.get("area");
    const keep = new URLSearchParams();
    if (area) keep.set("area", area);
    for (const facet of ["status", "kind"]) {
      const v = url.searchParams.get(facet);
      if (v) keep.set(facet, v);
    }
    for (const [k, v] of url.searchParams) {
      if (k === "view" || k === "viewer" || k.startsWith("vf.")) keep.set(k, v);
    }
    const qs = keep.toString();
    const target = qs ? `/program?${qs}#projects-pool` : "/program#projects-pool";
    return new Response(null, { status: 302, headers: { Location: target } });
  }
  // #1279: briefs need the latest comment per item, guaranteed per ref —
  // the bulk comments() read can truncate an item's latest comment away
  // in a busy batch, so prefer the source's per-ref latest read.
  // #1282 slice 1: one read-only inbox over every disagreement the facets
  // report. The page and its JSON twin are the same projection, assembled
  // once per request — they cannot disagree about what disagrees.
  if (path === "/conflicts" || path === "/api/conflicts.json") {
    const inbox = await assembleConflicts();
    if (path === "/api/conflicts.json") return json(inbox);
    return html(renderConflicts(inbox, prov(), provDetail()));
  }

  if (path === "/api/projects.json") {
    const [pool, census, poolAvailable, audit] = await Promise.all([
      source.projects().catch(() => []),
      source.coverageCensus ? source.coverageCensus().catch(() => null) : Promise.resolve(null),
      source.projectsAvailable ? source.projectsAvailable().catch(() => false) : Promise.resolve(true),
      source.projectAudit ? source.projectAudit().catch(() => null) : Promise.resolve(null),
    ]);
    const coverage = await coverageSnapshot(census, pool, poolAvailable);
    // #1279: the JSON twin of the brief panels on the pool cards — the
    // identical brief objects, built with comment activity folded in.
    const [briefComments, orchestration] = await Promise.all([
      pool.length ? briefCommentsFor(pool.map((p) => p.ref)) : Promise.resolve([]),
      source.orchestration().catch(() => null),
    ]);
    const orchestrationStatus = source.diagnostics?.().orchestration.state;
    const briefs = buildBriefs(pool, { comments: briefComments, orchestration, orchestrationStatus });
    const resolved = await resolveView(url);
    const outPool = resolved ? pool.filter((it) => itemMatchesView(poolItemLike(it), resolved.filters)) : pool;
    const outRefs = new Set(outPool.map((item) => item.ref));
    return json({
      project: "developing-today#4",
      items: outPool,
      // Build with the full pool so a filtered-out blocker can still have
      // its state resolved, but only expose briefs for the visible items.
      briefs: Object.fromEntries([...briefs].filter(([ref]) => outRefs.has(ref))),
      audit,
      designBoards: source.designBoards ? await source.designBoards().catch(() => []) : [],
      coverage: coverage.current,
      coverageBaseline: coverage.baseline,
    });
  }

  if (path === "/program" || path === "/api/program.json") {
    // #1046 + #1121: /program is the projects index. Editorial content
    // from src/program.ts (the pm-dashboard doc) and the project
    // registry in src/project-config.ts; the live half derived from
    // the same Source the other views read (items / projects /
    // roadmap / orchestration), each facet failing soft on its own.
    // #5082: the live-source fan-out rides the programSources cache
    // (TTL + single-flight); the local-store reads and everything
    // view-dependent below stay per-request.
    const [{ items, pool, rows, orchestration, census, poolAvailable }, formOpts, actionsByObject] = await Promise.all([
      programSources(),
      commentFormOpts(),
      actionsStore.byObject(),
    ]);
    // #1276 (Codex P2, PR #4013): /projects redirects here with the
    // view parameters preserved, but this handler never resolved the
    // view — apply it to the pool before rendering, exactly as
    // /api/projects.json does, so shared/default views filter the
    // HTML page (and its JSON twin) too.
    const resolved = await resolveView(url);
    const viewPool = resolved ? pool.filter((it) => itemMatchesView(poolItemLike(it), resolved.filters)) : pool;
    const data = {
      ...buildProgramData(items, viewPool, orchestration),
      projects: buildProjectSummaries(PROJECT_CONFIGS, viewPool, rows),
      designBoards: source.designBoards ? await source.designBoards().catch(() => []) : [],
    };
    if (path === "/api/program.json") return json(data);
    const coverage = await coverageSnapshot(census, pool, poolAvailable);
    // #1279: the pool cards on /program carry the same comment-backed
    // briefs as the JSON twin — building them here, not the comment-less
    // fallback projectsBodyHtml would derive on its own.
    const briefs = buildBriefs(pool, {
      comments: pool.length ? await briefCommentsFor(pool.map((p) => p.ref)) : [],
      orchestration,
      orchestrationStatus: source.diagnostics?.().orchestration.state,
    });
    return html(
      renderProgram(data, items, prov(), provDetail(), {
        pool: viewPool,
        designBoards: data.designBoards,
        area: url.searchParams.get("area"),
        status: url.searchParams.get("status"),
        kind: url.searchParams.get("kind"),
        audit: source.projectAudit ? await source.projectAudit().catch(() => null) : null,
        orchestration,
        coverage,
        actionsByObject,
        identities: formOpts.identities,
        briefs,
        forecasts: (await allForecastContracts(viewPool)).byRef,
        // #4830: the Projects page (merged into /program, #1497)
        // renders the #4829 twin as charts, computed as the endpoint
        // computes it.
        flowAnalytics: computeFlowAnalytics(await itemHistoryStore.readAll().catch(() => [])),
        // #4824: spend beside progress on the pool cards.
        spend: await loadSpendCards(),
      }),
    );
  }

  const programProjectMatch = /^\/program\/([\w-]+)$/.exec(path);
  if (programProjectMatch) {
    const config = projectConfigById(programProjectMatch[1]);
    if (!config) {
      return html(
        `<h1>Not found</h1><p class="muted">no project with id <code>${esc(programProjectMatch[1])}</code> in the registry (<code>src/project-config.ts</code>) — <a href="/program">back to the projects index</a></p>`,
        404,
      );
    }
    const [items, pool, rows, orchestration, formOpts, actionsByObject] = await Promise.all([
      source.items(allProjectRefs()).catch(() => new Map()),
      source.projects().catch(() => []),
      source.roadmap().catch(() => []),
      source.orchestration().catch(() => null),
      commentFormOpts(),
      actionsStore.byObject(),
    ]);
    // #9922: the id page's Path to Phase 12 panel — fetch the #1235
    // body snapshot when the source carries one, and resolve the
    // steps' refs into the items map so the join in render sees live
    // state for every step, not just the config's tracked refs.
    let phase12: Phase12Snapshot | null = null;
    if (config.id === "id" && source.phase12) {
      phase12 = await source.phase12().catch(() => null);
      if (phase12) {
        const stepRefs = parsePhase12Steps(phase12.body).map((s) => `${PHASE12_REPO}#${s.issueNumber}`);
        const missing = stepRefs.filter((r) => !items.has(r));
        if (missing.length) {
          const resolved = await source.items(missing).catch(() => new Map());
          for (const [k, v] of resolved) items.set(k, v);
        }
      }
    }
    // #9924: the id page's Build health panel — the Actions-API
    // snapshot, when the source carries one.
    let buildHealth: BuildHealthSnapshot | null = null;
    if (config.id === "id" && source.buildHealth) {
      buildHealth = await source.buildHealth().catch(() => null);
    }
    // #9921: the id page's Queue & fleet panel — the read-only
    // fleet-queue collector snapshot (#9920), when the source
    // carries one.
    let queueSnap: QueueSnapshot | null = null;
    if (config.id === "id" && source.queue) {
      queueSnap = await source.queue().catch(() => null);
    }
    // #9926: the id page's Phase 12 readiness panel — #534's body
    // (design checklist) and the F-item issues' timeline PR
    // cross-references, when the source carries them; the F-item
    // refs join the items map so the projection sees live states.
    let f12: { design: Phase12Snapshot | null; prRefs: Map<number, F12PrRef[]> | null } | null = null;
    if (config.id === "id") {
      const [design, prRefs] = await Promise.all([
        source.phase12Design ? source.phase12Design().catch(() => null) : Promise.resolve(null),
        source.f12PrRefs ? source.f12PrRefs().catch(() => null) : Promise.resolve(null),
      ]);
      f12 = { design, prRefs };
      if (phase12) {
        const fRefs = parseF12Items(phase12.body).map((s) => `${PHASE12_REPO}#${s.issueNumber}`);
        const missing = fRefs.filter((r) => !items.has(r));
        if (missing.length) {
          const resolved = await source.items(missing).catch(() => new Map());
          for (const [k, v] of resolved) items.set(k, v);
        }
      }
    }
    // #9923 step 3: the Phase tracker's F-item mapping snapshot
    // (planning/roadmaps/id.md via the source), when the source
    // carries one, with the mapped refs resolved into the items map
    // like the phase12 steps above.
    let phaseMapping: PhaseMappingSnapshot | null = null;
    if (config.id === "id" && source.phaseMapping) {
      phaseMapping = await source.phaseMapping().catch(() => null);
      if (phaseMapping) {
        const mapRefs = [...new Set(phaseMapping.entries.map((e) => `${PHASE_MAPPING_REPO}#${e.number}`))];
        const missing = mapRefs.filter((r) => !items.has(r));
        if (missing.length) {
          const resolved = await source.items(missing).catch(() => new Map());
          for (const [k, v] of resolved) items.set(k, v);
        }
      }
    }
    // #1108 remainder: the project's derived doc (roadmap
    // description + newest status record) from the checkout-reading
    // source, when it carries one; null keeps the registry text.
    const docTarget = projectDocTarget(config);
    const projectDoc = docTarget && source.projectDoc ? await source.projectDoc(docTarget).catch(() => null) : null;
    return html(
      renderProgramProject(
        buildProjectDetail(config, items, pool, rows, projectDoc),
        items,
        prov(),
        provDetail(),
        pool,
        phase12,
        buildHealth,
        f12,
        queueSnap,
        phaseMapping,
        {
          orchestration,
          actionsByObject,
          identities: formOpts.identities,
          orchestrationState: source.diagnostics?.().orchestration.state,
        },
      ),
    );
  }
  if (path === "/api/program-project.json") {
    const id = url.searchParams.get("id") ?? "";
    const config = projectConfigById(id);
    if (!config) return json({ error: "unknown project", id }, 404);
    const [items, pool, rows] = await Promise.all([
      source.items(allProjectRefs()).catch(() => new Map()),
      source.projects().catch(() => []),
      source.roadmap().catch(() => []),
    ]);
    const docTarget = projectDocTarget(config);
    const projectDoc = docTarget && source.projectDoc ? await source.projectDoc(docTarget).catch(() => null) : null;
    return json(buildProjectDetail(config, items, pool, rows, projectDoc));
  }

  if (path === "/health") {
    const view = await healthContext();
    return html(renderHealth(view, prov()));
  }

  if (path === "/pages") {
    return html(renderPages(prov(), pageRenderHealth()));
  }

  if (path === "/comments") {
    return html(renderCommentsQueue(await commentsStore.all(), prov()));
  }

  if (path === "/chat") {
    // #10035 S6: the threaded chat/session view over the prompt
    // store. list() with includeArchived returns every record; the
    // render hides archived unless ?archived=1 asks for them.
    const showArchived = url.searchParams.get("archived") === "1";
    return html(renderChat(await promptsStore.list({ includeArchived: true }), prov(), { showArchived }));
  }

  if (path === "/api/comments/queue.json") {
    const all = await commentsStore.all();
    return json({ comments: queuedComments(all), counts: commentKindCounts(all) });
  }

  const discussMatch = /^\/discuss\/(.+)$/.exec(path);
  if (discussMatch) {
    // #2748: a malformed percent-encoding is a JSON 400, never a raw
    // URIError out of the handler.
    const objectRef = decodePathSegment(discussMatch[1]);
    if (objectRef === null) {
      return json({ error: `malformed percent-encoding in path segment: ${discussMatch[1]}` }, 400);
    }
    const { title, githubRefs } = await discussTitle(objectRef);
    const thread = await commentsStore.forObject(objectRef);
    return html(renderDiscuss(objectRef, title, thread, await commentFormOpts(githubRefs), prov()));
  }

  if (path === "/api/overview.json") {
    return json(deriveOverview(await overviewContext()));
  }
  if (path === "/api/board.json") {
    const { board, items } = await boardContext();
    const showAllDone = url.searchParams.get("done") === "all";
    const resolved = await resolveView(url);
    if (!resolved) return jsonCompact(boardJsonBody(board, items, { showAllDone }));
    const pool = await source.projects().catch(() => [] as ProjectPoolItem[]);
    const poolByRef = new Map(pool.map((p) => [p.ref, p]));
    // Filter the policy-eligible board against the full item lookup
    // BEFORE the done-strip limit (#8058): limiting first dropped a
    // view's matching done cards outside the global newest 100.
    // boardJsonViewBody then applies the limit and restricts the
    // shipped items to the selected cards, so the response stays
    // bounded as #5082 requires — the full closure is only a lookup.
    const bounded = boardJsonViewBody(
      board,
      items,
      (b) => linkedLike({ ref: b.id, repo: b.repo, state: b.status }, b.links, items, poolByRef, resolved.filters),
      { showAllDone },
    );
    return jsonCompact({ board: bounded.board, items: bounded.items, view: resolved.view?.id ?? null });
  }
  if (path === "/api/releases.json") {
    // #9067 (design section 4/6): milestone close-out snapshots over
    // the collector's per-item milestone ingestion — program
    // milestones get retroactive counts, release milestones get the
    // close-out record (counts split closed vs carried-over, the
    // release date, and the description line).
    const pool = await source.projects().catch(() => [] as ProjectPoolItem[]);
    return json({
      generatedAt: new Date().toISOString(),
      source: "project pool milestone ingestion (#9067)",
      releases: buildReleaseSnapshots(pool),
    });
  }
  if (path === "/api/roadmap.json") {
    const { rows, items, reconciliation } = await roadmapContext();
    const pool = await source.projects().catch(() => [] as ProjectPoolItem[]);
    const genealogyRows = enrichRoadmapRows(rows, pool);
    if (url.searchParams.get("view") === "priorities") {
      genealogyRows.sort((a, b) => {
        const rank = (row: typeof a) => {
          const match = /#(\d+)$/.exec(row.priority ?? "");
          return match ? Number(match[1]) : Number.MAX_SAFE_INTEGER;
        };
        return rank(a) - rank(b) || a.title.localeCompare(b.title);
      });
    }
    const resolved = await resolveView(url);
    if (!resolved) {
      const annotatedRows = await Promise.all(
        genealogyRows.map(async (row) => ({
          ...row,
          annotations: await roadmapAnnotationsStore.forObject(row.subject),
        })),
      );
      return json({
        rows: annotatedRows,
        items: [...items.values()],
        drift: roadmapDriftReport(reconciliation),
        annotationAliases: await roadmapAnnotationsStore.aliasEntries(),
      });
    }
    const poolByRef = new Map(pool.map((p) => [p.ref, p]));
    // #8723: the same rule as the HTML twin -- items linked from a
    // kept row ship even when they fail the view themselves.
    const { rows: keptRows, items: keptItemMap } = filterRoadmapForView(rows, items, poolByRef, resolved.filters);
    const keptItems = [...keptItemMap.values()];
    const annotatedKeptRows = await Promise.all(
      enrichRoadmapRows(keptRows, pool).map(async (row) => ({
        ...row,
        annotations: await roadmapAnnotationsStore.forObject(row.subject),
      })),
    );
    return json({
      rows: annotatedKeptRows,
      items: keptItems,
      drift: roadmapDriftReport(reconciliation),
      annotationAliases: await roadmapAnnotationsStore.aliasEntries(),
      view: resolved.view?.id ?? null,
    });
  }
  if (path === "/api/quota.json") {
    // Extended in place for the hub (design §1.1): the pools/burn/choices
    // shape is unchanged for existing consumers (quota-guidance); the
    // hub sections and extractor health ride as additional top-level keys. Snapshot history
    // stays out of this twin — it is per-service, on the service twin.
    const [quota, usage] = await Promise.all([source.quota(), source.usage()]);
    // #1171: a stale read returns immediately with refresh state instead of
    // waiting on provider calls; the gatherer runs in the background.
    const refresh = quotaRefresher.request(quota);
    return json({
      ...quota,
      refresh,
      saas: usage.services,
      payg: usage.payg,
      charges: usage.charges,
      attention: usage.attention,
      domains: usage.domains,
      subscriptions: usage.subscriptions,

      uninstrumentedServices: usage.uninstrumentedServices,
      extractorHealth: {
        pools: quota.pools.map((pool) => ({
          id: pool.id,
          ...(pool.extractionHealth ?? { status: "unavailable", matched: null, reason: "not-recorded", count: 0 }),
        })),
        services: usage.services.map((service) => ({
          id: service.id,
          ...(service.extractionHealth ?? { status: "unavailable", matched: null, reason: "not-recorded", count: 0 }),
        })),
      },
    });
  }
  if (req.method === "POST" && path === "/api/quota/refresh") {
    const quota = await source.quota().catch(() => emptyQuotaView());
    const refresh = quotaRefresher.manual(quota);
    // generatedAt is the client's poll baseline (#6125): the manual
    // refresh is only done when quota-current.json advances past it.
    return json({ requested: refresh.triggered, refresh, generatedAt: quota.gatherer?.generatedAt ?? null });
  }
  if (path === "/api/quota/service.json") {
    const id = url.searchParams.get("id") ?? "";
    const usage = await source.usage();
    const svc = usage.services.find((s) => s.id === id);
    if (!svc) return json({ error: "unknown service", id }, 404);
    const history = usage.history[svc.id] ?? [];
    const paces = [...new Set(history.map((r) => r.metricId))]
      .map((metricId) => ({ metricId, pace: projectPace(history, metricId) }))
      .filter((p) => p.pace);
    return json({ service: svc, history, paces });
  }
  if (path === "/api/needs-drew.json") {
    // #1226: the Needs-Drew label census as data — open user/review +
    // user/escalated items across the four repos with per-source
    // status and freshness. 503 when this source cannot census, so a
    // consumer never mistakes "unavailable" for "nothing waiting".
    const census = (await source.needsDrew?.().catch(() => null)) ?? null;
    if (!census) return json({ error: "needs-drew census unavailable from this source" }, 503);
    return json(census);
  }
  if (path === "/api/attention.json") {
    // Slice 2: the flat array shape stays; entries are now current-only
    // (expired dropped) and tier-annotated (tier/drewReason/
    // orchestratorNote, source, parentId). #1148: the derived watchdog
    // tier items are appended in their TierItem shape (tier-annotated
    // needs-orchestrator) so machine consumers see the same queue the
    // HTML renders — the array shape itself is unchanged (Codex P2
    // on PR #2431). #3317: include non-session needs-drew items too,
    // matching the full queue rendered by the attention page.
    const { view, quotaItems } = await attentionContext();
    return json([
      ...view.all,
      ...view.drewExtra.map((i) => ({ ...i, tier: "needs-drew" as const })),
      ...view.watchdog.map((i) => ({ ...i, tier: "needs-orchestrator" as const })),
      // #1174 (Codex P2, PR #4695): the synthesized quota tasks ride along
      // in the same TierItem shape — this endpoint is the machine-first
      // interface the attention page advertises, so the quota tasks must
      // be discoverable here, not only in the HTML orchestrator tier.
      ...quotaItems.map((i) => ({ ...i, tier: "needs-orchestrator" as const })),
    ]);
  }
  if (path === "/api/coverage.json") {
    const { view, coverage } = await attentionContext();
    return json({ coverage, expired: view.expiredCount });
  }
  if (path === "/api/today.json") {
    const { today } = await todayContext();
    return json(today);
  }
  if (path === "/api/digest.json") {
    const kind = url.searchParams.get("kind") === "weekly" ? "weekly" : "daily";
    const date = url.searchParams.get("date");
    // Share the /digest handler's archive resolution (#5279 Codex): a
    // dated request returns the stored digest verbatim, so the machine
    // twin of a weekly or historical page is the digest it displays.
    if (date && /^\d{4}-\d{2}-\d{2}$/.test(date)) {
      const archived =
        (await readDigestArchive(cfg.digestsDir, date, kind)) ??
        (kind === "daily" ? null : await readDigestArchive(cfg.digestsDir, date, "daily"));
      if (archived) return json(archived);
      return json({ error: `no ${kind} digest archived for ${date}` }, 404);
    }
    return json(await digestContext(kind));
  }
  if (path === "/api/digests.json") {
    return json({ archives: await listDigestArchives(cfg.digestsDir) });
  }
  if (path === "/api/queue.json") {
    return json(await queueContext());
  }
  if (path === "/api/orchestration.json") {
    // #899: the orchestration job's tiers + goal measurements. Absent
    // means "not measured yet" — never an empty queue. #1148: the
    // aggregator-derived watchdog items ride along as an additive
    // field (the published section itself stays verbatim) so JSON
    // consumers can see derived queue items next to the published
    // tiers (Codex P2 on PR #2431).
    const section = await source.orchestration();
    const diagnosis = source.diagnostics?.().orchestration;
    const derivedWatchdogItems = await deriveWatchdogItems(diagnosis);
    return json(
      section
        ? { ...section, derivedWatchdogItems }
        : { status: "absent", reason: "no orchestration run has published yet", derivedWatchdogItems },
    );
  }
  if (path === "/sw.js") {
    // The real worker (#3676; source + §6 history in src/sw-source.ts).
    return serviceWorkerResponse();
  }
  if (path === "/manifest.webmanifest") {
    return new Response(JSON.stringify(PWA_MANIFEST, null, 2), {
      headers: { "content-type": "application/manifest+json; charset=utf-8", "cache-control": "no-cache" },
    });
  }
  const iconMatch = /^\/icons\/(icon-192\.png|icon-512\.png|icon-maskable-512\.png|apple-touch-icon\.png)$/.exec(path);
  if (iconMatch) {
    const file = Bun.file(new URL(`./assets/icons/${iconMatch[1]}`, import.meta.url));
    if (!(await file.exists())) return json({ error: "icon missing" }, 404);
    return new Response(file, { headers: { "content-type": "image/png", "cache-control": "public, max-age=86400" } });
  }
  if (path === "/assets/push-client.js") {
    const file = Bun.file(new URL("./assets/push-client.js", import.meta.url));
    if (!(await file.exists())) return json({ error: "asset missing" }, 404);
    return new Response(file, {
      headers: { "content-type": "text/javascript; charset=utf-8", "cache-control": "public, max-age=3600" },
    });
  }
  if (path === "/assets/prompt-badge.js") {
    const file = Bun.file(new URL("./assets/prompt-badge.js", import.meta.url));
    if (!(await file.exists())) return json({ error: "asset missing" }, 404);
    return new Response(file, {
      headers: { "content-type": "text/javascript; charset=utf-8", "cache-control": "public, max-age=3600" },
    });
  }
  if (path === "/assets/freshness-client.js") {
    const file = Bun.file(new URL("./assets/freshness-client.js", import.meta.url));
    if (!(await file.exists())) return json({ error: "asset missing" }, 404);
    return new Response(file, {
      headers: { "content-type": "text/javascript; charset=utf-8", "cache-control": "public, max-age=3600" },
    });
  }
  if (path === "/api/sw-probe") {
    // Design §6 step 2: pages report the registrations they see
    // (counted, no content); GET exposes the running summary.
    if (req.method === "POST") {
      try {
        const body = (await readJsonBody(req)) as { registrations?: unknown; evicted?: unknown };
        return json(recordSwProbe(body));
      } catch (err) {
        return json({ error: `invalid probe: ${err instanceof Error ? err.message : err}` }, 400);
      }
    }
    return json(swProbeSummary());
  }

  // Alert throat (#3676): subscription API + incident intake — server-alert-routes.ts (#13052).
  const alertIntakeResponse = await handleAlertIntakeRoutes(req, path);
  if (alertIntakeResponse) return alertIntakeResponse;

  // Needs-Drew answers (#8568) + decision-set APIs (#8766/#9335) — server-decision-routes.ts (#13052).
  const decisionResponse = await handleDecisionRoutes(req, url, path);
  if (decisionResponse) return decisionResponse;

  // --- Decision sets (#8766) -------------------------------------------
  // --- Decision-sets index (#8867) --------------------------------------
  // GET /decisions: every set in the store, counts computed live at
  // request time. The index carries no token of its own — it sits behind
  // the same access as the rest of the aggregator (tailnet origin /
  // Cloudflare Access), and each row links onward to the set's own
  // token-carrying page.
  if (req.method === "GET" && path === "/decisions") {
    return handleDecisionsIndex(url);
  }
  const decisionTailResponse = await handleDecisionTailRoutes(req, url, path);
  if (decisionTailResponse) return decisionTailResponse;

  // --- Decision-set aging operators (#14064) ---------------------------
  // Dismiss and sweep sit behind the alerts-intake bearer (operator
  // surface, same as the alert writes): they close questions without
  // a per-set token, so no token path may reach them.
  if (req.method === "POST" && path === "/api/decisions/dismiss") {
    if (!(await alertsIntakeAuthorized(req))) return json({ error: "unauthorized" }, 401);
    let body: { setIds?: unknown };
    try {
      body = (await readJsonBody(req)) as typeof body;
    } catch {
      return json({ error: "invalid dismiss body" }, 400);
    }
    if (!Array.isArray(body.setIds) || body.setIds.length > 512) {
      return json({ error: "setIds must be an array of up to 512 set ids" }, 400);
    }
    const setIds = body.setIds.filter((x): x is string => typeof x === "string" && x.length > 0);
    const result = await dismissDecisionSets(cfg.decisionsFile, setIds);
    return json(result);
  }
  if (req.method === "POST" && path === "/api/decisions/sweep") {
    if (!(await alertsIntakeAuthorized(req))) return json({ error: "unauthorized" }, 401);
    let body: { answeredElsewhere?: unknown };
    try {
      body = (await readJsonBody(req)) as typeof body;
    } catch {
      return json({ error: "invalid sweep body" }, 400);
    }
    if (!Array.isArray(body.answeredElsewhere) || body.answeredElsewhere.length > 1024) {
      return json({ error: "answeredElsewhere must be an array of up to 1024 entries" }, 400);
    }
    const answeredElsewhere = (body.answeredElsewhere as Array<Record<string, unknown>>)
      .filter((e) => e && typeof e === "object" && typeof e.qid === "string" && e.qid)
      .map((e) => ({
        qid: e.qid as string,
        ...(typeof e.optionId === "string" ? { optionId: e.optionId } : {}),
        ...(typeof e.optionLabel === "string" ? { optionLabel: e.optionLabel } : {}),
        ...(typeof e.freeText === "string" ? { freeText: e.freeText } : {}),
        ...(typeof e.comment === "string" ? { comment: e.comment } : {}),
      }));
    const result = await sweepDecisionSets(cfg.decisionsFile, answeredElsewhere);
    return json(result);
  }
  const alertActionMatch = /^\/api\/alerts\/([\w-]+)\/(ack|close)$/.exec(path);
  if (req.method === "POST" && alertActionMatch) {
    return handleAlertAction(alertActionMatch);
  }
  // Per-incident read (#12602): the incident plus its full ledger
  // event history — C6's detail path for the healthHook incident
  // records stored on the opened/closed event details.
  const alertReadMatch = /^\/api\/alerts\/([\w-]+)$/.exec(path);
  if (req.method === "GET" && alertReadMatch) {
    return handleAlertRead(alertReadMatch);
  }
  if (path === "/alerts" || path === "/api/alerts.json") {
    return handleAlertsList(path, prov);
  }
  if (path === "/alerts/preferences" || path === "/api/alerts/preferences.json") {
    return handleAlertPreferences(path, prov);
  }
  if (path === "/notifications" || path === "/api/notifications.json") {
    return handleNotificationsPage(path, prov);
  }
  if (path === "/api/health") {
    // #1267: prime first — health's answer must not depend on
    // request order. `sections` carries the structured diagnosis
    // (#1140) beside the provenance strings for machine consumers.
    // #1147 extends the shape in place (services/feed/pages/jobs
    // added from the same assembly /health renders; the existing
    // status/source/sections keys are unchanged).
    const [view, hostHealth] = await Promise.all([
      healthContext(),
      // #3989 criterion 6: expose the collector's active host and the
      // newest verified backup generation (date / finish time / hash /
      // destinations) on the existing machine health surface. A failed
      // or unreadable generation remains explicit; never synthesize a
      // healthy restore point from missing data.
      readHostHealthSnapshot(cfg.hostHealthFile).catch(() => null),
    ]);
    const body: Record<string, unknown> = { status: "ok", source: prov(), version: view.version };
    body.hostHealth = hostHealth
      ? { host: hostHealth.host, backup: hostHealth.backup }
      : { host: null, backup: null, state: "unavailable" };
    const diag = source.diagnostics?.();
    if (diag) body.sections = diag;
    body.services = view.services.map((s) => ({
      id: s.def.id,
      state: s.state,
      ok: s.ok,
      detail: s.detail,
      ...(s.probedAt ? { probedAt: s.probedAt } : {}),
    }));
    body.feed = {
      generatedAt: view.feed.generatedAt,
      fileMtime: view.feed.fileMtime,
      sections: view.feed.sections,
    };
    body.quotaGatherer = view.quotaGatherer;
    body.pages = view.pages.map((p) => ({ route: p.route, lastRenderedAt: p.lastRenderedAt, ok: p.ok }));
    body.jobs = view.jobs;
    body.comments = view.comments;
    body.conflicts = view.conflicts;
    // #1203: the ready-for-agent harness facet — backend readiness,
    // repository settings, work-item counts and GraphQL errors reach the
    // machine surface too, not only the rendered page.
    body.rfa = view.rfa;
    // #3976: push health (journal + reconciler state) reaches the
    // machine surface too, from the same assembly /health renders.
    body.push = view.push;
    body.degraded = view.degraded;
    return json(body);
  }

  const itemJsonMatch = /^\/api\/item\/(.+)\.json$/.exec(path);
  const itemMatch = /^\/item\/(.+)$/.exec(path);
  const matchedItemPath = itemJsonMatch ?? itemMatch;
  if (matchedItemPath) {
    // #2748: same guard as /discuss — JSON 400 on malformed encoding.
    const encodedId = matchedItemPath[1];
    const id = decodePathSegment(encodedId);
    if (id === null) {
      return json({ error: `malformed percent-encoding in path segment: ${encodedId}` }, 400);
    }
    const [board, rows] = await Promise.all([source.board().catch(() => []), source.roadmap().catch(() => [])]);
    const roadmapRow = rows.find((r) => r.subject === id);
    const subject = board.find((b) => b.id === id) ?? roadmapRow;
    if (subject) {
      const title = subject.title;
      const links = subject.links;
      const refs = allRefs([subject]);
      const activityRefs = [...new Set([id, ...refs].map((ref) => ref.replace(/^gh:/i, "")))];
      const activityQuery = new URLSearchParams();
      for (const ref of activityRefs) activityQuery.append("ref", ref);
      const [itemEvents, itemGaps] = await Promise.all([
        eventsStore.list({ refs: activityRefs }),
        activityStore.gaps(),
      ]);
      const [items, comments, allComments, records, pool, orchestration, allActions, roadmapAnnotations] =
        await Promise.all([
          source.items(refs).catch(() => new Map()),
          source.comments(refs).catch(() => []),
          commentsStore.all(),
          source.registry().catch(() => []),
          source.projects().catch(() => []),
          source.orchestration().catch(() => null),
          actionsStore.all(),
          roadmapRow ? roadmapAnnotationsStore.forObject(id) : Promise.resolve(undefined),
        ]);
      const briefRefs = refs.filter((ref) => pool.some((item) => item.ref === ref));
      const briefComments = briefRefs.length ? await briefCommentsFor(briefRefs) : [];
      const briefs = buildBriefs(pool, {
        comments: briefComments,
        orchestration,
        orchestrationStatus: source.diagnostics?.().orchestration.state,
      });
      // Dashboard comments for this item live on its own subject ref
      // and on each linked GitHub ref (gh:<ref>) — one merged thread.
      const wanted = new Set([id, ...refs.map((r) => ghObjectRef(r))]);
      const dashComments = allComments.filter((c) => wanted.has(c.objectRef));
      const sessions = records.filter((r) => r.taskRefs.some((ref) => refs.includes(ref)));
      // #1154: when one of the item's refs is in the Project pool, the
      // item page carries the status-request control for that pool item.
      const pooled = pool.find((p) => refs.includes(p.ref));
      const genealogy = pooled ? deriveGenealogy(pooled, pool) : null;
      const epicRollup = pooled?.kind === "Epic" ? buildEpicRollups(pool).get(pooled.ref) : undefined;
      const dispatchStates = await readyAgentDispatchStates(items);
      if (itemJsonMatch) {
        return json({
          subject,
          linkedItems: [...items.values()],
          dispatchStates,
          ...(genealogy ? { ancestors: genealogy.ancestors, release: genealogy.release } : {}),
        });
      }
      const extras = {
        dashComments,
        ...(roadmapRow ? { roadmapAnnotations } : {}),
        formRef: id,
        serviceLinks: itemServiceLinks(subject),
        formOpts: await commentFormOpts(refs),
        sessions,
        actions: allActions.filter((a) => wanted.has(a.objectRef)),
        ...(pooled
          ? {
              statusRequest: {
                objectRef: ghObjectRef(pooled.ref),
                targetRef: pooled.ref,
                currentStatus: pooled.status ?? null,
              },
            }
          : {}),
        ...(epicRollup ? { epicRollup } : {}),
        ...(genealogy ? { genealogySentence: genealogy.sentence } : {}),
        briefs,
        ...(dispatchStates.length ? { dispatchStates } : {}),
        activity: {
          events: itemEvents,
          globalHref: `/events?${activityQuery.toString()}`,
          gaps: itemGaps,
          coverage: ACTIVITY_SOURCE_COVERAGE,
        },
      };
      return html(renderItem(title, id, links, items, comments, prov(), extras));
    }
    // #4969 (Codex on PR #5283): an unresolved /api/item/<id>.json id
    // gets the structured JSON 404, not the HTML page response.
    if (itemJsonMatch) return notFoundResponse(path);
    return html(
      `<h1>Not found</h1><p class="muted">no board item or roadmap row with subject <code>${esc(id)}</code></p>`,
      404,
    );
  }

  return notFoundResponse(path);
}

/**
 * #1147's render registry: every successful HTML page render is
 * recorded against its manifest route (subroutes count toward their
 * parent page), so /health can report each page's real last-render
 * time — measured from actual traffic, never self-probed. The mapping
 * lives with the manifest (#1135) so nav reachability and render
 * attribution can never disagree.
 */

async function fetchHandler(req: Request, remoteAddress?: string): Promise<Response> {
  // #1211: the top-level error boundary. A throwing handler degrades
  // to a styled 500 page (JSON for /api) instead of rejecting the
  // fetch; the render-health record below still runs, accounting the
  // failure as an unsuccessful render.
  let res: Response;
  try {
    res = await dashboardResponseWithinDeadline(req, () => handler(req, remoteAddress));
  } catch (err) {
    console.error(`aggregator: handler failed for ${req.method} ${req.url}:`, err);
    res = errorBoundaryResponse(new URL(req.url).pathname, err);
  }
  if (req.method === "GET") {
    const route = manifestRouteFor(new URL(req.url).pathname);
    if (route && (res.headers.get("content-type") ?? "").includes("text/html")) {
      recordRender(route, res.status < 400);
    }
  }
  const publicHost = process.env.PM_AGG_CONSOLE_HOST?.trim() || "pm.security.cab";
  return addConsoleShortcut(res, req, publicHost);
}

/** Per-socket state for the /ws/prompt/:id channel (#10033 S3). */
interface PromptWsData {
  promptId: string;
  session?: PromptWsSession;
}

/**
 * #10033 S3: upgrade a /ws/prompt/:id request. An unknown prompt id is
 * a plain 404 JSON response (no upgrade attempted); a refused upgrade
 * is a 400. On success the websocket handlers below own the socket and
 * this fetch returns nothing further.
 */
async function promptWsUpgrade(
  req: Request,
  server: Bun.Server<PromptWsData>,
  pathname: string,
): Promise<Response | undefined> {
  const id = decodeURIComponent(pathname.slice("/ws/prompt/".length));
  const prompt = await promptsStore.get(id);
  if (!prompt) return json({ error: "prompt not found" }, 404);
  if (server.upgrade(req, { data: { promptId: id } })) return undefined;
  return json({ error: "websocket upgrade failed" }, 400);
}

// One server per bind address (PM_AGG_HOSTS). Deployment on the
// laptop binds loopback + the tailnet address, like Pad/Grist — no
// public route (Drew, 2026-10-25: public only behind a password).
const servers = cfg.hosts.map((hostname) =>
  Bun.serve<PromptWsData>({
    hostname,
    port: cfg.port,
    // Phase B §13.1: slow fragments (overview/board/queue renders) exceed
    // Bun's 10s default idle timeout and the patch response gets killed
    // mid-render; 60s gives the heavy contexts room without going unbounded.
    idleTimeout: 60,
    fetch(req, server) {
      const pathname = new URL(req.url).pathname;
      // #10033 S3: the sync-ws channel upgrades ahead of the ordinary
      // request pipeline — it is a socket, not a rendered response.
      if (pathname.startsWith("/ws/prompt/")) return promptWsUpgrade(req, server, pathname);
      if (req.method === "POST" && pathname === "/api/prompt") {
        // A sync-block submission may legitimately hold its request for
        // the full configured answer wait (120s default), past the 60s
        // socket idle timeout above; lift this request's cap to cover
        // the wait plus margin (Bun caps the value at 255).
        server.timeout(req, Math.min(255, Math.ceil(cfg.promptSyncTimeoutMs / 1000) + 5));
      }
      // #9608: the peer address rides along so the outage-queueing
      // path can require a stack-local origin (design #1253 §6).
      return fetchHandler(req, server.requestIP(req)?.address);
    },
    websocket: {
      open(ws) {
        const session = createPromptWsSession({
          store: promptsStore,
          promptId: ws.data.promptId,
          socket: {
            send: (data) => {
              ws.send(data);
            },
            close: (code, reason) => {
              ws.close(code, reason);
            },
          },
          timeoutMs: cfg.promptSyncTimeoutMs,
        });
        ws.data.session = session;
        void session.start();
      },
      message() {
        // The channel is server-to-client only; inbound frames are ignored.
      },
      close(ws) {
        ws.data.session?.stop();
      },
    },
  }),
);

for (const server of servers) {
  console.log(`aggregator listening on http://${server.hostname}:${server.port} (source: ${cfg.source})`);
}

// #5082: warm the quota-analytics aggregates once at boot, and the
// page assemblies with them. The aggregates warm-up no longer calls
// buildQuotaAnalytics synchronously: the first build's unsliced scans
// measure ~12 s of uninterrupted event-loop time over the 21 GB
// ledger, and a request that arrived during the old warm-up queued
// behind the whole window — the first /api/program.json after a
// restart measured 26.4 s against ~0.03 s warm (2026-11-13
// follow-up). primeAttemptAggregates fills the same cache through
// the day-sliced stale-while-revalidate refresh instead, so the loop
// stays responsive; later builds ride that cache as before. Since
// #2795 the aggregates come from the OpenCode server's rollup API, so
// there is no synchronous cold build at all: a /quota request landing
// before the prime settles shows the section as unavailable.
// programSources joins the warmed assemblies in the same follow-up:
// it was the one major assembly left out, so /program's first
// post-restart request always paid its cold fan-out on the request
// path, inside whatever loop backlog the boot window left behind.
setTimeout(() => {
  void (async () => {
    try {
      primeAttemptAggregates(readRouteApiTarget(cfg.opencodeServerStateFile));
      await Promise.allSettled([
        boardContext(),
        overviewContext(),
        todayContext(),
        programSources(),
        commentIdentities(),
      ]);
      // #9781: warm the briefs' latest comments for the program pool
      // too — the board briefs read a subset of the same refs, so one
      // prime covers both pages, and neither page's first post-restart
      // request pays the per-ref projector fan-out.
      const warmed = await programSources().catch(() => null);
      if (warmed?.pool.length) {
        await briefCommentsFor(warmed.pool.map((p) => p.ref));
      }
    } catch {
      // fail-soft: the first real request retries its own assembly
    }
  })();
}, 0);

// Cross-process publishers get a separate loopback-only listener; never
// add this port to PM_AGG_HOSTS or expose it through the Cloudflare tunnel.
const freshnessIpcServer = Bun.serve({
  hostname: "127.0.0.1",
  port: cfg.freshnessPort,
  fetch: createFreshnessIpcHandler(freshnessStore),
});
console.log(`freshness publisher listening on http://${freshnessIpcServer.hostname}:${freshnessIpcServer.port}`);

// #3976: start the anti-entropy loop in live mode — one pass now,
// then the 5-minute cadence. Fixtures mode has no journal to reconcile.
if (cfg.source === "live") {
  void freshnessReconciler.reconcile().catch(() => undefined);
  freshnessReconciler.start();
}

// #8570: pm-core consolidation — in live mode the quota gatherer and
// the fleet-feed collector also run as in-process schedulers on the
// seam's cadences (quota 15 minutes, fleet-feed 5 minutes), ahead of
// the systemd timer-unit cutover (a later slice of this issue).
// Shutdown is tied to process signals inside the wiring.
if (cfg.source === "live") {
  startPmCoreSchedulers({ feedFile: cfg.feedFile });
}

// Collector feeds are replaced on a ~5-minute cadence. Land their
// snapshot records from a server-owned writer tick rather than from
// usage() so GET /quota and /quota/service/* never require store writes.
const SNAPSHOT_INGEST_INTERVAL_MS = 5 * 60_000;
if (source.ingestSnapshots) {
  let snapshotIngestRunning = false;
  const ingestSnapshots = async (): Promise<void> => {
    if (snapshotIngestRunning) return;
    snapshotIngestRunning = true;
    try {
      await source.ingestSnapshots?.();
    } catch {
      // Feed and store failures are retried on the next tick; reads stay up.
    } finally {
      snapshotIngestRunning = false;
    }
  };
  void ingestSnapshots(); // recover the latest feed immediately on startup
  setInterval(() => void ingestSnapshots(), SNAPSHOT_INGEST_INTERVAL_MS);
}

// The hot-list consumer is a server-owned writer just like snapshot
// ingestion: it runs immediately on startup and then periodically, never
// as a side effect of rendering a page. A failed sweep leaves the remote
// checkpoint untouched and is retried on the next tick.
const GITHUB_HOT_LIST_INTERVAL_MS = 60_000;
if (githubHotListConsumer) {
  const hotListConsumer = githubHotListConsumer;
  let githubSyncRunning = false;
  const syncGithubHotList = async (): Promise<void> => {
    if (githubSyncRunning) return;
    githubSyncRunning = true;
    try {
      await hotListConsumer.sync();
      if (githubOutcomeConsumer) {
        const outcome = await githubOutcomeConsumer.sync();
        if (outcome.error) {
          console.error(`[activity] WARNING: GitHub outcome sync ${outcome.state}: ${outcome.error}`);
        }
      }
    } catch (error) {
      console.error(
        `[activity] WARNING: GitHub hot-list worker failed: ${error instanceof Error ? error.message : String(error)}`,
      );
    } finally {
      githubSyncRunning = false;
    }
  };
  void syncGithubHotList();
  const timer = setInterval(() => void syncGithubHotList(), GITHUB_HOT_LIST_INTERVAL_MS);
  timer.unref?.();
} else {
  void activityStore.setSourceStatus(unconfiguredGithubStatus()).catch((error) => {
    console.error(
      `[activity] WARNING: could not persist unconfigured GitHub source status: ${error instanceof Error ? error.message : String(error)}`,
    );
  });
}

// SLO sampler (#9988): one sample per facet per minute, appended to the
// journal from the same health view /health renders. Live mode only.
const SLO_SAMPLE_INTERVAL_MS = 60_000;
if (cfg.source !== "fixtures") {
  let sloSampling = false;
  const sampleSlo = async (): Promise<void> => {
    if (sloSampling) return;
    sloSampling = true;
    try {
      const view = await healthContext(false);
      await sloJournal.append(sampleSloFacets(view, Date.now()));
    } catch (error) {
      console.error(`[slo] WARNING: SLO sample failed: ${error instanceof Error ? error.message : String(error)}`);
    } finally {
      sloSampling = false;
    }
  };
  void sampleSlo();
  const timer = setInterval(() => void sampleSlo(), SLO_SAMPLE_INTERVAL_MS);
  timer.unref?.();
}

// #9991: the §4 escalation sinks — the real journal/feed/push surfaces
// escalation.ts's delivery step was built against. Journal: the
// aggregator's append-only watchdog journal (the SLO journal file,
// where the watchdog heartbeat already rides). Feed: the activity
// stream #1148's watchdog reports into. Push: the page-health-push
// path — sendPush fanned out over the alerts store's subscriptions
// with the incident class enabled, sends logged like the test-push
// path (design §6 question 1 is unanswered, so the interim channel is
// the existing one, per #9991's acceptance). The push sink receives
// only the page payload, so its sends-ledger rows carry the stable
// objectRef below; the journal and feed entries carry the facet.
const ESCALATION_PAGE_OBJECT_REF = "watchdog-escalation";
const escalationSink: EscalationSink = {
  journal: async (entry) => {
    await sloJournal.append([escalationJournalLine(entry)]);
  },
  feedEvent: async (entry) => {
    const result = await recordActivity([escalationDelivery(entry)]);
    if (result.failed > 0) {
      throw new Error(`activity ingestion failed for ${result.failedIds.join(", ")}`);
    }
  },
  push: async (page) => {
    const store = await getAlertsStore();
    if (!store) return { ok: false, gone: false, error: "alerts store unavailable" };
    const subs = (await store.listSubscriptions()).filter((sub) => classEnabled(sub, "incident"));
    if (subs.length === 0) {
      return { ok: false, gone: false, error: "no push subscription has the incident class enabled" };
    }
    const payload: PushPayload = {
      class: "incident",
      objectRef: ESCALATION_PAGE_OBJECT_REF,
      url: `${cfg.publicOrigin}/alerts`,
      title: page.title,
      body: page.body,
    };
    let sent = 0;
    let lastError = "push was not delivered";
    for (const sub of subs) {
      const outcome = await sendPush({ endpoint: sub.endpoint, p256dh: sub.p256dh, auth: sub.auth }, payload);
      if (outcome.ok) {
        sent += 1;
        await store.logSend({ class: "incident", objectRef: ESCALATION_PAGE_OBJECT_REF, outcome: "sent" });
        await store.noteSubscriptionAlive(sub.endpoint);
      } else if (outcome.gone) {
        await store.logSend({ class: "incident", objectRef: ESCALATION_PAGE_OBJECT_REF, outcome: "gone" });
        await store.noteSubscriptionGone(sub.endpoint);
        lastError = outcome.error;
      } else {
        await store.logSend({ class: "incident", objectRef: ESCALATION_PAGE_OBJECT_REF, outcome: "error" });
        lastError = outcome.error;
      }
    }
    return sent > 0 ? { ok: true, status: 201 } : { ok: false, gone: false, error: lastError };
  },
};

const escalationTicker = createEscalationTicker({
  summaries: (now) => sloSummaries(now),
  // No remediation executor host exists yet (escalation-tick.ts's seam
  // note): the empty state is the stack's actual remediation
  // condition — nothing attempted, nothing in flight, no breaker open.
  remediationState: (now) => emptyState(now),
  sink: escalationSink,
});

// Watchdog derivation must not depend on dashboard traffic or push
// subscriptions. Its heartbeat is written only after deriveWatchdogItems
// completes health/audit reads and gap sync. Keep this timer independent
// of the alert engine and attention routes: their availability is not
// evidence that the watchdog itself ran.
const WATCHDOG_RUN_INTERVAL_MS = 5 * 60_000;
if (cfg.source === "live") {
  let watchdogRunInProgress = false;
  const runWatchdog = async (): Promise<void> => {
    if (watchdogRunInProgress) return;
    watchdogRunInProgress = true;
    try {
      // Refresh the live orchestration diagnosis before deriving cards.
      // A source failure is not a successful watchdog pass.
      await source.orchestration();
      await deriveWatchdogItems(source.diagnostics?.().orchestration, "scheduled");
      // #9991: the §4 escalation pass rides the same scheduled tick,
      // in its own try/catch — an escalation failure must not fail
      // the watchdog pass whose heartbeat deriveWatchdogItems wrote.
      try {
        await escalationTicker.tick();
      } catch (error) {
        console.error(
          `[watchdog] WARNING: escalation pass failed: ${error instanceof Error ? error.message : String(error)}`,
        );
      }
    } catch (error) {
      console.error(
        `[watchdog] WARNING: scheduled derivation failed: ${error instanceof Error ? error.message : String(error)}`,
      );
    } finally {
      watchdogRunInProgress = false;
    }
  };
  void runWatchdog();
  const timer = setInterval(() => void runWatchdog(), WATCHDOG_RUN_INTERVAL_MS);
  timer.unref?.();
}

// The alert throat's engine (#3676): the ~60s delivery/escalation
// tick (live mode only, fail-soft — see src/alerts.ts).
startAlertEngine(cfg, async (): Promise<NotificationSnapshot> => {
  const [
    attention,
    quota,
    health,
    actions,
    comments,
    needsDrewRaised,
    decisionSets,
    stackHealthDown,
    stackHealthStates,
    opencodeConfigText,
  ] = await Promise.all([
    attentionContext(),
    source.quota().catch(() => emptyQuotaView()),
    healthContext(false),
    actionsStore.all().catch(() => []),
    commentsStore.all().catch(() => []),
    listOpenNeedsDrew(cfg.needsDrewFile).catch(() => []),
    listOpenDecisionSets(cfg.decisionsFile).catch(() => []),
    listStackHealthDown(cfg.stackHealthFile).catch(() => []),
    readStackHealthStates(cfg.stackHealthFile).catch(() => ({})),
    // #8882: the section-risk join reads the deployed config's route
    // leaf sets; a missing/unreadable config yields no membership.
    readFile(cfg.opencodeConfigFile, "utf8").catch(() => ""),
  ]);
  const drewActivity = [
    ...actions
      .filter((action) => action.author.kind === "drew")
      .map((action) => ({ objectRef: action.objectRef, at: action.createdAt })),
    ...comments
      .filter((comment) => comment.author.kind === "drew")
      .map((comment) => ({ objectRef: comment.objectRef, at: comment.createdAt })),
  ];
  return {
    attention: attention.view,
    quota,
    health,
    drewActivity,
    soleDoorPoolIds: deriveSoleDoorPoolIds(quota),
    sectionPools: sectionPoolIdsFromConfig(
      opencodeConfigText,
      quota.pools.map((pool) => pool.id),
    ),
    needsDrewRaised,
    decisionSets,
    stackHealthDown,
    stackHealthStates,
  };
});

// The notify relay's laptop half (#8569 Phase 2 step 1): ~60s
// heartbeats to the pm-notify edge Durable Object + the
// subscription mirror that authorizes its sends. A no-op until
// PM_AGG_NOTIFY_RELAY_URL is set (the deploy is a separate leg),
// and fail-soft once running — the edge is a mirror of this
// stack, never a dependency of it.
startNotifyRelay(cfg, () => getAlertsStore());
