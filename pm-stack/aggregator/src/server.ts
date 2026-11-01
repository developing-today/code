// The aggregator's HTTP server. Read-mostly: pages are server-rendered
// HTML and /api/*.json serve the same data machine-first. Slice 3 (#906)
// adds the app's first writes — dashboard comments (POST /api/comments
// and its triage/github-result companions); the triage write path
// (#1152/#1154/#1156) adds dashboard actions (POST /api/actions and
// its result companion) on the same pattern. Everything underneath
// (GitHub, Kaneo, Grist) stays read-only from here — the fleet jobs
// apply actions; the dashboard only records them.
import { stat } from "node:fs/promises";
import { decodePathSegment } from "./path-segment.ts";
import { ActionsStore, validateNewAction } from "./actions.ts";
import { checkActionAgainstItem, effectiveNeedsOrder, effectiveStandingAuto, findTierItem } from "./tier-items.ts";
import { CommentsStore, validateNewComment } from "./comments.ts";
import {
  computeCoverage,
  CoverageSnapshotStore,
  nextCoverageSnapshot,
  type CoverageSnapshotFile,
  type RepoCensus,
} from "./coverage.ts";
import { loadConfig } from "./config.ts";
import { itemServiceLinks } from "./service-links.ts";
import { loadFeed } from "./feed.ts";
import {
  buildHealth,
  pageRenderHealth,
  recordRender,
  summarizeItemCoverage,
  type HealthView,
  type ItemsCoverage,
} from "./health.ts";
import { manifestRouteFor } from "./pages.ts";
import { watchdogTierItems, WATCHDOG_PROBE_SERVICE_IDS } from "./watchdog.ts";
import { allRefs, buildAttention, deriveToday, projectPace, type RoadmapGrain } from "./derive.ts";
import {
  reconcileRoadmap,
  roadmapDriftReport,
  unavailableRoadmapReconciliation,
  type RoadmapReconciliation,
} from "./roadmap-reconcile.ts";
import type { DashComment, ProjectPoolItem, RefKey } from "./model.ts";
import { buildProgramData, programRefs } from "./program.ts";
import {
  PROJECT_CONFIGS,
  allProjectRefs,
  buildProjectDetail,
  buildProjectSummaries,
  projectConfigById,
} from "./project-config.ts";
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
  renderHealth,
  renderItem,
  deriveOverview,
  renderOverview,
  renderOverviewBody,
  renderOverviewShell,
  renderPages,
  renderProgram,
  renderProgramProject,
  renderQuota,
  renderQueue,
  renderQueueBody,
  renderQueueShell,
  renderRoadmap,
  renderServiceDetail,
  renderSessions,
  renderToday,
  renderTodayBody,
  renderTodayShell,
  type CommentFormOpts,
} from "./render.ts";
import { timingSafeEqual } from "node:crypto";
import {
  getAlertsStore,
  sendTestPush,
  startAlertEngine,
  validateNewIncident,
  validateNewSubscription,
} from "./alerts.ts";
import { recordSwProbe, swProbeSummary, VAPID_PUBLIC_KEY } from "./push.ts";
import { resolveBwsSecret } from "./secrets.ts";
import { SW_SOURCE } from "./sw-source.ts";
import { renderAlerts } from "./render.ts";
import { parseRegistryQuery, queryRegistry } from "./registry-view.ts";
import type { Source } from "./source.ts";
import { fixtureSource } from "./sources/fixtures.ts";
import { liveSource } from "./sources/live.ts";
import { buildQueueView } from "./queue-view.ts";

const cfg = loadConfig();
const commentsStore = new CommentsStore(cfg.commentsFile);
const coverageStore = new CoverageSnapshotStore(cfg.coverageSnapshotFile);

/** #1220: the coverage snapshot behind the /projects panel — computed
 * from the feed's items + the Project pool, rotated weekly by
 * nextCoverageSnapshot, persisted only when it actually changes. */
async function coverageSnapshot(
  census: RepoCensus[] | null,
  pool: ProjectPoolItem[],
  poolAvailable: boolean,
): Promise<CoverageSnapshotFile> {
  const stored = await coverageStore.read();
  const next = nextCoverageSnapshot(stored, computeCoverage(census, poolAvailable ? pool : null));
  const strip = (s: CoverageSnapshotFile | null): string =>
    s
      ? JSON.stringify({
          baseline: s.baseline,
          weekly: s.weekly ?? null,
          anchorAt: s.anchorAt ?? "",
          current: { ...s.current, generatedAt: "" },
        })
      : "";
  if (strip(next) !== strip(stored)) {
    await coverageStore.write(next).catch(() => {});
  }
  return next;
}
const actionsStore = new ActionsStore(cfg.actionsFile);

function pickSource(): Source {
  // #1290: the live source reads the standing delegation default from
  // the actions store's preference records when it stamps the
  // needs-user fallbacks onto tier items.
  return cfg.source === "live"
    ? liveSource(cfg, {
        standingAuto: async () => effectiveStandingAuto(await actionsStore.all()).standingAuto,
      })
    : fixtureSource();
}

const source = pickSource();

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
let lastPrimedAt = 0;
/** #1141: item coverage measured by the latest priming pass (below),
 *  from the same board items() resolution that sets facet provenance. */
let lastItemsCoverage: ItemsCoverage | null = null;
let priming: Promise<void> | null = null;
async function ensureProvenancePrimed(): Promise<void> {
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
async function healthContext(prime = true): Promise<HealthView> {
  // Priming exercises every facet so /health provenance never depends
  // on request order. Callers that only need the feed snapshot and the
  // cached probes (the attention path's watchdog assembly, #1148) pass
  // prime=false: a stalled unrelated facet (e.g. Grist's roadmap API
  // and its 8s timeout) must not delay attention rendering (Codex P2
  // on PR #2431). The registry age the watchdog reads comes from the
  // live source's lastFeed, which attention's own reads already load.
  if (prime) await ensureProvenancePrimed();
  const [feed, feedFileMtimeMs] = await Promise.all([
    loadFeed(cfg.feedFile).catch(() => null),
    stat(cfg.feedFile)
      .then((s) => s.mtimeMs)
      .catch(() => null),
  ]);
  return buildHealth({
    mode: cfg.source,
    cfg,
    feed,
    feedFileMtimeMs,
    diagnosis: source.diagnostics?.().orchestration,
    detail: source.provenanceDetail?.(),
    itemsCoverage: lastItemsCoverage,
    comments: await commentsStore.pendingCounts().catch(() => undefined),
  });
}

/** Author identities for the comment form: standing + registry sessions. */
async function commentFormOpts(githubRefs: RefKey[] = []): Promise<CommentFormOpts> {
  const records = await source.registry().catch(() => []);
  return { githubRefs, identities: identitiesFromRecords(records) };
}

/** Resolve a discuss-page title for any object ref, best-effort. */
async function discussTitle(objectRef: string): Promise<{ title: string; githubRefs: RefKey[] }> {
  if (objectRef.startsWith("gh:")) {
    const ref = objectRef.slice(3);
    const items = await source.items([ref]).catch(() => new Map());
    return { title: items.get(ref)?.title ?? `Comments — ${ref}`, githubRefs: [ref] };
  }
  if (objectRef.startsWith("session:")) {
    const id = objectRef.slice("session:".length);
    const records = await source.registry().catch(() => []);
    const rec = records.find((r) => r.id === id);
    return { title: `Comments — ${rec?.label ?? id}`, githubRefs: rec?.taskRefs ?? [] };
  }
  if (objectRef.startsWith("orch:")) {
    const id = objectRef.slice("orch:".length);
    const section = await source.orchestration().catch(() => null);
    const item = [...(section?.tiers.needsDrew ?? []), ...(section?.tiers.needsOrchestrator ?? [])].find(
      (i) => i.id === id,
    );
    return {
      title: `Comments — ${item?.title ?? id}`,
      githubRefs: item?.ref ? [item.ref] : [],
    };
  }
  const [board, rows] = await Promise.all([source.board().catch(() => []), source.roadmap().catch(() => [])]);
  const item = board.find((b) => b.id === objectRef);
  if (item) return { title: `Comments — ${item.title}`, githubRefs: item.links.map((l) => l.ref) };
  const row = rows.find((r) => r.subject === objectRef);
  if (row) return { title: `Comments — ${row.title}`, githubRefs: row.links.map((l) => l.ref) };
  return { title: `Comments — ${objectRef}`, githubRefs: [] };
}

async function readJsonBody(req: Request): Promise<unknown> {
  const text = await req.text();
  if (text.length > 100_000) throw new Error("body too large");
  return JSON.parse(text);
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body, null, 2), {
    status,
    headers: { "content-type": "application/json; charset=utf-8" },
  });
}

function html(body: string, status = 200): Response {
  return new Response(body, { status, headers: { "content-type": "text/html; charset=utf-8" } });
}

// The real service worker (#3676 — design 2026-11-01T03-54-00Z §6
// step 2): push + notificationclick handlers, with the kill switch's
// eviction job carried forward per the §6 fallback (cache purge on
// activate in the worker; foreign-registration unregister in the
// page-side probe script). Worker source + history: src/sw-source.ts.
function serviceWorkerResponse(): Response {
  return new Response(SW_SOURCE, {
    headers: { "content-type": "text/javascript; charset=utf-8", "cache-control": "no-cache" },
  });
}

// --- Alert throat (#3676) -------------------------------------------------

/** Bearer check for POST /api/alerts: the intake token is resolved
 * by name from Bitwarden (pm-aggregator.alerts-intake-token) and
 * compared in constant time; its value is never logged. */
async function alertsIntakeAuthorized(req: Request): Promise<boolean> {
  const header = req.headers.get("authorization") ?? "";
  const presented = header.startsWith("Bearer ") ? header.slice("Bearer ".length).trim() : "";
  if (!presented) return false;
  const expected = await resolveBwsSecret("pm-aggregator.alerts-intake-token");
  if (!expected) return false;
  const a = Buffer.from(presented);
  const b = Buffer.from(expected);
  return a.length === b.length && timingSafeEqual(a, b);
}

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** The PWA manifest (#3315 slice a, landed with #3676): start_url is
 * the attention view per the design; icons incl. maskable + iOS. */
const PWA_MANIFEST = {
  name: "PM Aggregator",
  short_name: "PM Stack",
  description: "The PM-stack aggregator: needs-you queues, quotas, health \u2014 and the alert throat.",
  start_url: "/attention",
  scope: "/",
  display: "standalone",
  background_color: "#0e1117",
  theme_color: "#161b22",
  icons: [
    { src: "/icons/icon-192.png", sizes: "192x192", type: "image/png", purpose: "any" },
    { src: "/icons/icon-512.png", sizes: "512x512", type: "image/png", purpose: "any" },
    { src: "/icons/icon-maskable-512.png", sizes: "512x512", type: "image/png", purpose: "maskable" },
  ],
};

async function boardContext() {
  const board = await source.board();
  const refs = allRefs(board);
  const items = await source.items(refs);
  return { board, items };
}

async function roadmapContext() {
  const rows = await source.roadmap();
  const refs = allRefs(rows);
  const [items, poolResult] = await Promise.all([
    source.items(refs),
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
  return { rows, items, reconciliation };
}

// #1148: the watchdog derives NEEDS ORCHESTRATOR items from the
// current diagnoses (feed age + probes via the health assembly —
// its probe cache keeps this cheap — plus the orchestration
// diagnosis and registry facet age). Live mode only: fixtures mode
// has no real inputs to watch. Derived per request, never stored,
// so recovery clears the cards on the next render. The health
// assembly here is deliberately UNPRIMED (see healthContext): the
// watchdog needs only the feed file, the cached probes, and the
// registry age from the source's already-loaded feed.
async function deriveWatchdogItems(diagnosis?: ReturnType<NonNullable<Source["diagnostics"]>>["orchestration"]) {
  if (cfg.source !== "live") return [];
  const health = await healthContext(false);
  return watchdogTierItems({
    now: Date.now(),
    feedAgeMs: health.feed.ageMs,
    feedGeneratedAt: health.feed.generatedAt,
    ...(diagnosis ? { orchestration: diagnosis } : {}),
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
  });
}

async function attentionContext() {
  const [feed, summary, coverage] = await Promise.all([source.attention(), source.orchestration(), source.coverage()]);
  // Read the diagnosis AFTER orchestration() has run: the live source
  // recomputes it on every orchestration read (#1139).
  const diagnosis = source.diagnostics?.().orchestration;
  const watchdog = await deriveWatchdogItems(diagnosis);
  return { view: buildAttention(feed, summary, Date.now(), diagnosis, watchdog), coverage };
}

/**
 * One Datastar patch-elements event (#1444). The fragment root
 * carries its own id + data-on-load attributes, so "outer" mode
 * replaces the shell's placeholder with a self-refreshing container.
 */
function ssePatch(selector: string, elementHtml: string): string {
  const lines = elementHtml
    .split("\n")
    .map((l) => `data: elements ${l}`)
    .join("\n");
  return `event: datastar-patch-elements\ndata: mode outer\ndata: selector ${selector}\n${lines}\n\n`;
}

function sseResponse(body: string): Response {
  return new Response(body, {
    headers: { "content-type": "text/event-stream; charset=utf-8", "cache-control": "no-cache" },
  });
}

/** The render inputs the board page and its fragment share. */
async function boardRender() {
  const { board, items } = await boardContext();
  const [pool, formOpts, actionsByObject, counts] = await Promise.all([
    source.projects().catch(() => []),
    commentFormOpts(),
    actionsStore.byObject(),
    commentsStore.counts(),
  ]);
  const act = {
    actionsByObject,
    poolByRef: new Map(pool.map((p) => [p.ref, p])),
    identities: formOpts.identities,
  };
  return { board, items, counts, act };
}

/** The render inputs the attention page and its fragments share:
 *  the view, the action/comment stores grouped by object, and the
 *  effective Needs-you order preference (#1456). */
async function attentionRender() {
  const { view, coverage } = await attentionContext();
  const [formOpts, actionsByObject, commentsByObject, counts, allActions] = await Promise.all([
    commentFormOpts(),
    actionsStore.byObject(),
    commentsStore.byObject(),
    commentsStore.counts(),
    actionsStore.all(),
  ]);
  const order = effectiveNeedsOrder(allActions);
  const act = {
    actionsByObject,
    identities: formOpts.identities,
    commentsByObject,
    commentOpts: formOpts,
    needsOrder: order.order,
    needsOrderPending: order.pending,
  };
  return { view, coverage, counts, act };
}

async function todayContext() {
  const [{ board, items }, { rows }, { view, coverage }, quota, summary] = await Promise.all([
    boardContext(),
    roadmapContext(),
    attentionContext(),
    source.quota(),
    source.orchestration(),
  ]);
  const diagnosis = source.diagnostics?.().orchestration;
  return { today: deriveToday(board, rows, items, summary, new Date(), diagnosis), view, coverage, quota };
}

async function queueContext() {
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
async function overviewContext() {
  const [{ view }, pool, quota, health, { board }, actionsByObject] = await Promise.all([
    attentionContext(),
    source.projects().catch(() => [] as ProjectPoolItem[]),
    source.quota(),
    healthContext(),
    boardContext(),
    actionsStore.byObject().catch(() => new Map()),
  ]);
  return { attention: view, pool, quota, health, board, actionsByObject };
}

const GRAINS = new Set<string>(["quarter", "month", "week", "3d", "today"]);

async function handler(req: Request): Promise<Response> {
  const url = new URL(req.url);
  const path = url.pathname;
  const prov = () => source.provenance();
  const provDetail = () => source.provenanceDetail?.();

  // --- Author identities for the on-demand picker (#1179) ------------------
  // List pages render author pickers with the standing identities only;
  // the full standing + per-session list is fetched once, on first use.
  if (req.method === "GET" && path === "/api/author-identities") {
    const records = await source.registry().catch(() => []);
    return json({ identities: identitiesFromRecords(records) });
  }

  // --- Dashboard comment writes (#906 §2) --------------------------------
  if (req.method === "POST" && path === "/api/comments") {
    try {
      const parsed = validateNewComment(await readJsonBody(req));
      if (!parsed.ok) return json({ error: parsed.error }, 400);
      // #1509: a persistence failure is a server error, not bad input -
      // only the validation/body-parse path above earns a 400.
      let comment;
      try {
        comment = await commentsStore.add(parsed.value);
      } catch (err) {
        return json({ error: `comment persistence failed: ${err instanceof Error ? err.message : err}` }, 500);
      }
      return json({ comment }, 201);
    } catch (err) {
      return json({ error: `invalid comment: ${err instanceof Error ? err.message : err}` }, 400);
    }
  }
  if (req.method === "POST" && path === "/api/comments/triage") {
    try {
      const body = (await readJsonBody(req)) as { ids?: unknown };
      const ids = Array.isArray(body.ids) ? body.ids.filter((x): x is string => typeof x === "string") : [];
      return json({ triaged: await commentsStore.markTriaged(ids) });
    } catch (err) {
      return json({ error: `invalid triage: ${err instanceof Error ? err.message : err}` }, 400);
    }
  }
  if (req.method === "POST" && path === "/api/comments/github-result") {
    try {
      const body = (await readJsonBody(req)) as { id?: unknown; url?: unknown; error?: unknown };
      if (typeof body.id !== "string") return json({ error: "id required" }, 400);
      const updated = await commentsStore.markGithubResult(body.id, {
        url: typeof body.url === "string" ? body.url : undefined,
        error: typeof body.error === "string" ? body.error : undefined,
      });
      return updated ? json({ comment: updated }) : json({ error: "no such github comment" }, 404);
    } catch (err) {
      return json({ error: `invalid result: ${err instanceof Error ? err.message : err}` }, 400);
    }
  }
  // --- Dashboard action writes (#1152/#1154/#1156) -----------------------
  // The triage write path: forms POST DashActions here; the fleet
  // consumers (orchestration pass, Jules keeper) poll state=new and
  // report outcomes to /api/actions/result. The dashboard itself never
  // writes GitHub, the Project, or a session server.
  if (req.method === "POST" && path === "/api/actions") {
    try {
      const parsed = validateNewAction(await readJsonBody(req));
      if (!parsed.ok) return json({ error: parsed.error }, 400);
      // #1291/#1293: checks that need the currently published item —
      // option/survey membership, required-question coverage, proposal
      // identity, the access/policy auto ban. Enforced only when the
      // aggregator actually holds a section; with none, the store
      // stays the permissive boundary of the triage slice and the
      // consumer rejects what it cannot apply.
      if (parsed.value.kind === "triage") {
        const section = await source.orchestration().catch(() => null);
        if (section) {
          const problem = checkActionAgainstItem(
            parsed.value,
            findTierItem(section, parsed.value.objectRef),
            Date.now(),
          );
          if (problem) return json({ error: problem }, 400);
        }
      }
      // #1509: persistence failure -> 5xx (see /api/comments above).
      let added;
      try {
        added = await actionsStore.add(parsed.value);
      } catch (err) {
        return json({ error: `action persistence failed: ${err instanceof Error ? err.message : err}` }, 500);
      }
      const { action, created } = added;
      return json(created ? { action } : { action, deduped: true }, created ? 201 : 200);
    } catch (err) {
      return json({ error: `invalid action: ${err instanceof Error ? err.message : err}` }, 400);
    }
  }
  if (req.method === "POST" && path === "/api/actions/result") {
    try {
      const body = (await readJsonBody(req)) as { id?: unknown; outcome?: unknown; byRun?: unknown; note?: unknown };
      if (typeof body.id !== "string") return json({ error: "id required" }, 400);
      if (body.outcome !== "applied" && body.outcome !== "rejected") {
        return json({ error: 'outcome must be "applied" | "rejected"' }, 400);
      }
      if (typeof body.byRun !== "string" || body.byRun.trim() === "") return json({ error: "byRun required" }, 400);
      const updated = await actionsStore.markResult(body.id, {
        outcome: body.outcome,
        byRun: body.byRun,
        note: typeof body.note === "string" ? body.note : undefined,
      });
      return updated ? json({ action: updated }) : json({ error: "no such action" }, 404);
    } catch (err) {
      return json({ error: `invalid result: ${err instanceof Error ? err.message : err}` }, 400);
    }
  }
  if (path === "/api/actions.json") {
    const object = url.searchParams.get("object");
    const kind = url.searchParams.get("kind");
    const state = url.searchParams.get("state");
    let list = await actionsStore.all();
    if (object) list = list.filter((a) => a.objectRef === object);
    if (kind) list = list.filter((a) => a.kind === kind);
    if (state) list = list.filter((a) => a.state === state);
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
    const wrapped = `<div id="overview-main" data-frag data-on-load="@get('/frag/overview')" data-on-dashrefresh__window="@get('/frag/overview')">${renderOverviewBody(deriveOverview(data))}</div>`;
    return sseResponse(ssePatch("#overview-main", wrapped));
  }
  if (path === "/board") {
    const { board, items, counts, act } = await boardRender();
    return html(renderBoard(board, items, prov(), counts, provDetail(), act, { showAllDone: url.searchParams.get("done") === "all" }));
  }
  if (path === "/frag/board") {
    const { board, items, counts, act } = await boardRender();
    const wrapped = `<div id="board-main" data-frag data-on-load="@get('/frag/board')" data-on-dashrefresh__window="@get('/frag/board')">${renderBoardBody(board, items, counts, act)}</div>`;
    return sseResponse(ssePatch("#board-main", wrapped));
  }

  if (path === "/today") {
    if (url.searchParams.get("full") !== "1") {
      return html(renderTodayShell(prov(), provDetail()));
    }
    const { today, view, coverage, quota } = await todayContext();
    return html(renderToday(today, view, quota, coverage, prov(), provDetail()));
  }
  if (path === "/frag/today") {
    const { today, view, coverage, quota } = await todayContext();
    const wrapped = `<div id="today-main" data-frag data-on-load="@get('/frag/today')" data-on-dashrefresh__window="@get('/frag/today')">${renderTodayBody(today, view, quota, coverage)}</div>`;
    return sseResponse(ssePatch("#today-main", wrapped));
  }

  if (path === "/queue") {
    if (url.searchParams.get("full") !== "1") return html(renderQueueShell(prov(), provDetail()));
    return html(renderQueue(await queueContext(), prov(), provDetail()));
  }
  if (path === "/frag/queue") {
    const view = await queueContext();
    const wrapped = `<div id="queue-main" data-frag data-on-load="@get('/frag/queue')" data-on-dashrefresh__window="@get('/frag/queue')">${renderQueueBody(view)}</div>`;
    return sseResponse(ssePatch("#queue-main", wrapped));
  }

  if (path === "/roadmap") {
    const { rows, items, reconciliation } = await roadmapContext();
    const g = url.searchParams.get("h") ?? "quarter";
    const grain: RoadmapGrain = GRAINS.has(g) ? (g as RoadmapGrain) : "quarter";
    return html(renderRoadmap(rows, items, prov(), grain, await commentsStore.counts(), provDetail(), reconciliation));
  }

  if (path === "/quota") {
    const [quota, usage] = await Promise.all([source.quota(), source.usage()]);
    return html(renderQuota(quota, prov(), usage, provDetail()));
  }

  const serviceMatch = /^\/quota\/service\/([\w-]+)$/.exec(path);
  if (serviceMatch) {
    const usage = await source.usage();
    const svc = usage.services.find((s) => s.id === serviceMatch[1]);
    if (svc) return html(renderServiceDetail(svc, usage.history[svc.id] ?? [], prov(), provDetail()));
    return html(
      `<h1>Not found</h1><p class="muted">no SaaS service with id <code>${serviceMatch[1]}</code> — <a href="/quota">back to Usage &amp; billing</a></p>`,
    );
  }

  if (path === "/attention") {
    // #1444: shell first; header/queues/sessions stream in as
    // fragments. ?full=1 keeps the full server render (no-JS path).
    if (url.searchParams.get("full") !== "1") {
      return html(renderAttentionShell(prov(), provDetail()));
    }
    const { view, coverage, counts, act } = await attentionRender();
    return html(renderAttention(view, coverage, prov(), counts, provDetail(), act));
  }
  const attnFragMatch = /^\/frag\/attention\/(header|queues|sessions)$/.exec(path);
  if (attnFragMatch) {
    const section = attnFragMatch[1] as "header" | "queues" | "sessions";
    const { view, coverage, counts, act } = await attentionRender();
    const sections = attentionSections(view, coverage, counts, act);
    return sseResponse(ssePatch(`#attn-${section}`, attentionFragment(section, sections[section])));
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

  if (path === "/dns") {
    return html(renderDns(await source.dns(), prov(), provDetail()));
  }
  if (path === "/api/dns.json") {
    const section = await source.dns();
    return json(section ?? { status: "absent", reason: "no DNS register section in the fleet feed yet" });
  }

  if (path === "/aa") {
    return html(renderAa(await source.aa(), prov()));
  }
  if (path === "/api/aa.json") {
    const section = await source.aa();
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
    const target = area ? `/program?area=${encodeURIComponent(area)}#projects-pool` : "/program#projects-pool";
    return new Response(null, { status: 302, headers: { Location: target } });
  }
  if (path === "/api/projects.json") {
    const [pool, census, poolAvailable] = await Promise.all([
      source.projects().catch(() => []),
      source.coverageCensus ? source.coverageCensus().catch(() => null) : Promise.resolve(null),
      source.projectsAvailable ? source.projectsAvailable().catch(() => false) : Promise.resolve(true),
    ]);
    const coverage = await coverageSnapshot(census, pool, poolAvailable);
    return json({
      project: "developing-today#4",
      items: pool,
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
    const [items, pool, rows, orchestration, formOpts, actionsByObject, census, poolAvailable] = await Promise.all([
      source.items(programRefs()).catch(() => new Map()),
      source.projects().catch(() => []),
      source.roadmap().catch(() => []),
      source.orchestration().catch(() => null),
      commentFormOpts(),
      actionsStore.byObject(),
      source.coverageCensus ? source.coverageCensus().catch(() => null) : Promise.resolve(null),
      source.projectsAvailable ? source.projectsAvailable().catch(() => false) : Promise.resolve(true),
    ]);
    const data = {
      ...buildProgramData(items, pool, orchestration),
      projects: buildProjectSummaries(PROJECT_CONFIGS, pool, rows),
    };
    if (path === "/api/program.json") return json(data);
    const coverage = await coverageSnapshot(census, pool, poolAvailable);
    return html(
      renderProgram(data, items, prov(), provDetail(), {
        pool,
        area: url.searchParams.get("area"),
        orchestration,
        coverage,
        actionsByObject,
        identities: formOpts.identities,
      }),
    );
  }

  const programProjectMatch = /^\/program\/([\w-]+)$/.exec(path);
  if (programProjectMatch) {
    const config = projectConfigById(programProjectMatch[1]);
    if (!config) {
      return html(
        `<h1>Not found</h1><p class="muted">no project with id <code>${programProjectMatch[1]}</code> in the registry (<code>src/project-config.ts</code>) — <a href="/program">back to the projects index</a></p>`,
        404,
      );
    }
    const [items, pool, rows] = await Promise.all([
      source.items(allProjectRefs()).catch(() => new Map()),
      source.projects().catch(() => []),
      source.roadmap().catch(() => []),
    ]);
    return html(renderProgramProject(buildProjectDetail(config, items, pool, rows), items, prov(), provDetail()));
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
    return json(buildProjectDetail(config, items, pool, rows));
  }

  if (path === "/health") {
    const view = await healthContext();
    return html(renderHealth(view, prov()));
  }

  if (path === "/pages") {
    return html(renderPages(prov(), pageRenderHealth()));
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
    return json({ board, items: [...items.values()] });
  }
  if (path === "/api/roadmap.json") {
    const { rows, items, reconciliation } = await roadmapContext();
    return json({ rows, items: [...items.values()], drift: roadmapDriftReport(reconciliation) });
  }
  if (path === "/api/quota.json") {
    // Extended in place for the hub (design §1.1): the pools/burn/choices
    // shape is unchanged for existing consumers (quota-guidance); the
    // hub sections ride as additional top-level keys. Snapshot history
    // stays out of this twin — it is per-service, on the service twin.
    const [quota, usage] = await Promise.all([source.quota(), source.usage()]);
    return json({
      ...quota,
      saas: usage.services,
      payg: usage.payg,
      charges: usage.charges,
      attention: usage.attention,
      domains: usage.domains,
      subscriptions: usage.subscriptions,
    });
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
  if (path === "/api/attention.json") {
    // Slice 2: the flat array shape stays; entries are now current-only
    // (expired dropped) and tier-annotated (tier/drewReason/
    // orchestratorNote, source, parentId). #1148: the derived watchdog
    // tier items are appended in their TierItem shape (tier-annotated
    // needs-orchestrator) so machine consumers see the same queue the
    // HTML renders — the array shape itself is unchanged (Codex P2
    // on PR #2431).
    const { view } = await attentionContext();
    return json([...view.all, ...view.watchdog.map((i) => ({ ...i, tier: "needs-orchestrator" as const }))]);
  }
  if (path === "/api/coverage.json") {
    const { view, coverage } = await attentionContext();
    return json({ coverage, expired: view.expiredCount });
  }
  if (path === "/api/today.json") {
    const { today } = await todayContext();
    return json(today);
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

  // --- Alert throat (#3676): subscription API -------------------------
  if (path === "/api/push/subscriptions") {
    const store = await getAlertsStore();
    if (!store) return json({ error: "alerts store unavailable" }, 503);
    if (req.method === "GET") return json({ subscriptions: await store.listSubscriptions() });
    if (req.method === "POST") {
      try {
        const parsed = validateNewSubscription(await readJsonBody(req));
        if (!parsed.ok) return json({ error: parsed.error }, 400);
        return json({ subscription: await store.upsertSubscription(parsed.value) }, 201);
      } catch (err) {
        return json({ error: `invalid subscription: ${err instanceof Error ? err.message : err}` }, 400);
      }
    }
    if (req.method === "DELETE") {
      try {
        const body = (await readJsonBody(req)) as { endpoint?: unknown };
        if (typeof body.endpoint !== "string") return json({ error: "endpoint required" }, 400);
        const deleted = await store.deleteSubscription(body.endpoint);
        return deleted ? json({ deleted: true }) : json({ error: "no such subscription" }, 404);
      } catch (err) {
        return json({ error: `invalid delete: ${err instanceof Error ? err.message : err}` }, 400);
      }
    }
    return json({ error: "method not allowed" }, 405);
  }

  if (path === "/api/push/test") {
    if (req.method !== "POST") return json({ error: "method not allowed" }, 405);
    const store = await getAlertsStore();
    if (!store) return json({ error: "alerts store unavailable" }, 503);
    const summary = await sendTestPush(store, cfg.publicOrigin);
    return json(summary);
  }

  // --- Alert throat (#3676): incident intake + ack/close --------------
  if (req.method === "POST" && path === "/api/alerts") {
    if (!(await alertsIntakeAuthorized(req))) return json({ error: "unauthorized" }, 401);
    const store = await getAlertsStore();
    if (!store) return json({ error: "alerts store unavailable" }, 503);
    try {
      const parsed = validateNewIncident(await readJsonBody(req));
      if (!parsed.ok) return json({ error: parsed.error }, 400);
      const { incident, created } = await store.postIncident(parsed.value);
      return json({ incident, created }, created ? 201 : 200);
    } catch (err) {
      return json({ error: `invalid incident: ${err instanceof Error ? err.message : err}` }, 400);
    }
  }
  const alertActionMatch = /^\/api\/alerts\/([\w-]+)\/(ack|close)$/.exec(path);
  if (req.method === "POST" && alertActionMatch) {
    const id = alertActionMatch[1];
    if (!UUID_RE.test(id)) return json({ error: "incident id must be a uuid" }, 400);
    const store = await getAlertsStore();
    if (!store) return json({ error: "alerts store unavailable" }, 503);
    const result = alertActionMatch[2] === "ack" ? await store.ackIncident(id) : await store.closeIncident(id);
    if (!result.incident) return json({ error: "no such incident", id }, 404);
    return json({ incident: result.incident, transitioned: result.transitioned });
  }
  if (path === "/alerts" || path === "/api/alerts.json") {
    const store = await getAlertsStore();
    if (!store) {
      return path === "/alerts"
        ? html(
            renderAlerts(
              { available: false, incidents: [], events: [], subscriptions: [], ackWindowMin: cfg.alerts.ackWindowMin },
              prov(),
              VAPID_PUBLIC_KEY,
            ),
          )
        : json({ error: "alerts store unavailable" }, 503);
    }
    try {
      const [incidents, events, subscriptions] = await Promise.all([
        store.listIncidents(),
        store.recentEvents(),
        store.listSubscriptions(),
      ]);
      if (path === "/api/alerts.json") return json({ incidents, events });
      return html(
        renderAlerts(
          { available: true, incidents, events, subscriptions, ackWindowMin: cfg.alerts.ackWindowMin },
          prov(),
          VAPID_PUBLIC_KEY,
        ),
      );
    } catch {
      return path === "/alerts"
        ? html(
            renderAlerts(
              { available: false, incidents: [], events: [], subscriptions: [], ackWindowMin: cfg.alerts.ackWindowMin },
              prov(),
              VAPID_PUBLIC_KEY,
            ),
          )
        : json({ error: "alerts store unavailable" }, 503);
    }
  }
  if (path === "/api/health") {
    // #1267: prime first — health's answer must not depend on
    // request order. `sections` carries the structured diagnosis
    // (#1140) beside the provenance strings for machine consumers.
    // #1147 extends the shape in place (services/feed/pages/jobs
    // added from the same assembly /health renders; the existing
    // status/source/sections keys are unchanged).
    const view = await healthContext();
    const body: Record<string, unknown> = { status: "ok", source: prov(), version: view.version };
    const diag = source.diagnostics?.();
    if (diag) body.sections = diag;
    body.services = view.services.map((s) => ({
      id: s.def.id,
      state: s.state,
      ok: s.ok,
      detail: s.detail,
      ...(s.probedAt ? { probedAt: s.probedAt } : {}),
    }));
    body.feed = { generatedAt: view.feed.generatedAt, fileMtime: view.feed.fileMtime };
    body.pages = view.pages.map((p) => ({ route: p.route, lastRenderedAt: p.lastRenderedAt, ok: p.ok }));
    body.jobs = view.jobs;
    body.comments = view.comments;
    body.degraded = view.degraded;
    return json(body);
  }

  const itemMatch = /^\/item\/(.+)$/.exec(path);
  if (itemMatch) {
    // #2748: same guard as /discuss — JSON 400 on malformed encoding.
    const id = decodePathSegment(itemMatch[1]);
    if (id === null) {
      return json({ error: `malformed percent-encoding in path segment: ${itemMatch[1]}` }, 400);
    }
    const [board, rows] = await Promise.all([source.board(), source.roadmap()]);
    const subject = board.find((b) => b.id === id) ?? rows.find((r) => r.subject === id);
    if (subject) {
      const title = subject.title;
      const links = subject.links;
      const notes = "repo" in subject ? subject.notes : undefined;
      const refs = allRefs([subject]);
      const [items, comments, allComments, records, pool, allActions] = await Promise.all([
        source.items(refs),
        source.comments(refs),
        commentsStore.all(),
        source.registry().catch(() => []),
        source.projects().catch(() => []),
        actionsStore.all(),
      ]);
      // Dashboard comments for this item live on its own subject ref
      // and on each linked GitHub ref (gh:<ref>) — one merged thread.
      const wanted = new Set([id, ...refs.map((r) => ghObjectRef(r))]);
      const dashComments = allComments.filter((c) => wanted.has(c.objectRef));
      const sessions = records.filter((r) => r.taskRefs.some((ref) => refs.includes(ref)));
      // #1154: when one of the item's refs is in the Project pool, the
      // item page carries the status-request control for that pool item.
      const pooled = pool.find((p) => refs.includes(p.ref));
      const extras = {
        dashComments,
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
      };
      return html(renderItem(title, id, links, notes, items, comments, prov(), extras));
    }
    return html(`<h1>Not found</h1><p class="muted">no board item or roadmap row with subject <code>${id}</code></p>`);
  }

  return json({ error: "not found", path }, 404);
}

/**
 * #1147's render registry: every successful HTML page render is
 * recorded against its manifest route (subroutes count toward their
 * parent page), so /health can report each page's real last-render
 * time — measured from actual traffic, never self-probed. The mapping
 * lives with the manifest (#1135) so nav reachability and render
 * attribution can never disagree.
 */

async function fetchHandler(req: Request): Promise<Response> {
  const res = await handler(req);
  if (req.method === "GET") {
    const route = manifestRouteFor(new URL(req.url).pathname);
    if (route && (res.headers.get("content-type") ?? "").includes("text/html")) {
      recordRender(route, res.status < 400);
    }
  }
  return res;
}

// One server per bind address (PM_AGG_HOSTS). Deployment on the
// laptop binds loopback + the tailnet address, like Pad/Grist — no
// public route (Drew, 2026-10-25: public only behind a password).
const servers = cfg.hosts.map((hostname) =>
  Bun.serve({
    hostname,
    port: cfg.port,
    fetch: fetchHandler,
  }),
);

for (const server of servers) {
  console.log(`aggregator listening on http://${server.hostname}:${server.port} (source: ${cfg.source})`);
}

// The alert throat's engine (#3676): the ~60s delivery/escalation
// tick (live mode only, fail-soft — see src/alerts.ts).
startAlertEngine(cfg);
