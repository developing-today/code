// Server wiring: config, stores, source selection, consumers and the
// source-side sinks. Split out of server.ts (#13052) — the stores and
// the Source are process-wide singletons created once at import.
import { createQuotaRefresher, systemdTrigger } from "./quota-refresh.ts";
import { ActionsStore, type NewAction } from "./actions.ts";
import { effectiveStandingAuto } from "./tier-items.ts";
import { CommentsStore } from "./comments.ts";
import { PromptsStore } from "./prompts.ts";
import { ViewsStore } from "./views.ts";
import { EventsStore, emitEvent } from "./events.ts";
import { FreshnessStore, type FreshnessInput } from "./freshness.ts";
import { fleetFeedRevisionReader, FreshnessReconciler } from "./freshness-reconcile.ts";
import { startPmCoreSchedulers } from "./pm-core-wiring.ts";
import { createPmCoreProjectorReader } from "./pm-core-projector-reader.ts";
import { IngestionStore, WatchdogEpisodeStabilizer, ingestActivity, type ActivitySourceStatus } from "./activity.ts";
import { GithubHotListConsumer, unconfiguredGithubStatus } from "./github-hot-list.ts";
import { FileOutcomeBindingSource, GithubOutcomeConsumer } from "./outcome-consumer.ts";
import { OutcomeFactStore } from "./outcome-attribution.ts";
import { RoadmapAnnotationsStore } from "./roadmap-annotations.ts";
import { IdentityRegistryStore } from "./identity-registry.ts";
import { BreakGlassTokenStore } from "./break-glass.ts";
import {
  OutageQueueStore,
  confirmOutageWrite,
  isStackLocalAddress,
  outageQueueEligible,
  queueOutageWrite,
  rejectOutageWrite,
  type OutageQueuedWrite,
} from "./outage-queue.ts";
import { PromptAttachmentsStore } from "./prompt-attachments.ts";
import {
  computeCoverage,
  CoverageSnapshotStore,
  nextCoverageSnapshot,
  type CoverageSnapshotFile,
  type RepoCensus,
} from "./coverage.ts";
import { loadConfig } from "./config.ts";
import { getTursoStoreClient } from "./turso-stores.ts";
import { createTursoRemotes } from "./remote-stores.ts";
import { conflictInbox, type ConflictInbox } from "./conflicts.ts";
import { ConflictRulingsStore } from "./conflict-rulings.ts";
import { watchdogTierItems } from "./watchdog.ts";
import { buildAttention } from "./derive.ts";
import type { Comment, GhItem, ProjectPoolItem, QuotaView, RefKey, UsageView } from "./model.ts";
import { SpendRollupStore } from "./spend-rollups.ts";
import { buildSpendCards, type SpendCardsView } from "./spend-cards.ts";
import { identitiesFromRecords, type AuthorIdentity } from "./registry.ts";
import { type CommentFormOpts } from "./render.ts";
import type { Source } from "./source.ts";
import { fixtureSource } from "./sources/fixtures.ts";
import { liveSource } from "./sources/live.ts";
import { createDashboardResponseDeadline } from "./assembly-deadline.ts";
import { dispatchStateFor, fetchRfaState, RFA_NOT_PICKED_UP } from "./sources/ready-for-agent.ts";
import { TodayCache, TODAY_CACHE_TTL_MS } from "./today-cache.ts";
import { ItemHistoryStore } from "./item-history.ts";
import { SloJournal } from "./slo-journal.ts";
import { fireOpenCodeWorkingHook } from "./health-hook-fire.ts";
import { defaultReportRecordsFile } from "./report-records.ts";

// Cap `t` at `max` UTF-16 code units, but never leave half of a surrogate
// pair at the cut (#7740, mirroring the #7694-family capUnits guard): an
// astral character straddling the cap is dropped whole, so a persisted
// event field can never end in a lone high surrogate.
export function capUnits(t: string, max: number): string {
  if (t.length <= max) return t;
  let end = max;
  const before = t.charCodeAt(end - 1);
  const after = t.charCodeAt(end);
  if (before >= 0xd800 && before <= 0xdbff && after >= 0xdc00 && after <= 0xdfff) end--;
  return t.slice(0, end);
}

export const cfg = loadConfig();
// #10383: the bounded routes' last-good survives a restart on disk, so
// the first request after a deploy serves it instead of a 503.
export const dashboardResponseWithinDeadline = createDashboardResponseDeadline({ persistDir: cfg.lastGoodDir });
export const promptsStore = new PromptsStore(cfg.promptsFile);
export const promptAttachmentsStore = new PromptAttachmentsStore(cfg.promptAttachmentsDir);
export const roadmapAnnotationsStore = new RoadmapAnnotationsStore(cfg.roadmapAnnotationsFile);
export const coverageStore = new CoverageSnapshotStore(cfg.coverageSnapshotFile);
export const itemHistoryStore = new ItemHistoryStore(cfg.itemHistoryFile);
export const sloJournal = new SloJournal(cfg.sloJournalFile);
export const spendRollupStore = new SpendRollupStore(cfg.snapshotsDir);
/** #4824: the Projects/Roadmap spend-beside-progress view over the
 * #4823 rollup store. A failed read renders no spend block (the
 * store's absence is already the honest state — ingestion is a
 * separate slice); it never fabricates zeroed cards. */
export const loadSpendCards = async (): Promise<SpendCardsView | null> => {
  try {
    return buildSpendCards(await spendRollupStore.readAll());
  } catch {
    return null;
  }
};

export async function readyAgentDispatchStates(items: Map<RefKey, GhItem>) {
  const targets = [...items.values()].filter(
    (item) => item.labels.some((label) => label.toLowerCase() === "ready-for-agent") && /#\d+$/.test(item.ref),
  );
  if (!targets.length) return [];
  const observedAt = new Date().toISOString();
  if (cfg.source !== "live") {
    // #4969 (Codex on PR #5283): fixtures mode must never read the live
    // harness — deterministic fixture dispatch data, mirroring the
    // health.ts fixtures boundary. The canned set has no picked-up
    // work item, so every labelled fixture item is honestly
    // "not picked up".
    return targets.map((item) => ({
      ref: item.ref,
      state: RFA_NOT_PICKED_UP,
      provenance: { source: "ready-for-agent", observedAt, repository: item.ref.split("#")[0], mode: "fixtures" },
    }));
  }
  try {
    const state = await fetchRfaState(cfg.readyForAgent.baseUrl);
    return targets
      .map((item) => {
        const match = /^(.*)#(\d+)$/.exec(item.ref);
        if (!match) return null;
        return {
          ref: item.ref,
          state: dispatchStateFor(state, match[1], Number(match[2])) || RFA_NOT_PICKED_UP,
          provenance: { source: "ready-for-agent", observedAt, repository: match[1] },
        };
      })
      .filter((entry): entry is NonNullable<typeof entry> => entry !== null);
  } catch {
    return targets.map((item) => ({
      ref: item.ref,
      state: "unavailable",
      provenance: {
        source: "ready-for-agent",
        observedAt,
        repository: item.ref.split("#")[0],
        error: "harness read failed",
      },
    }));
  }
}

/** #1220: the coverage snapshot behind the /projects panel — computed
 * from the feed's items + the Project pool, rotated weekly by
 * nextCoverageSnapshot, persisted only when it actually changes. */
export async function coverageSnapshot(
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
// #6815: the four formerly laptop-local stores (actions, activity
// ingestion, usage snapshots, aa-last-good) live in Turso when
// PM_AGG_STORE_BACKEND=turso. Exactly one backend is active: with
// "file" (the default) no remote exists and no database call is made.
export const storeRemotes = cfg.storeBackend === "turso" ? createTursoRemotes(getTursoStoreClient) : null;
export const actionsStore = new ActionsStore(cfg.actionsFile, undefined, storeRemotes?.actions);
// #1744 (Move 1, step 2): dashboard comments join the same switch —
// canonical in public.shared_comments under the turso backend.
export const commentsStore = new CommentsStore(cfg.commentsFile, undefined, storeRemotes?.comments);
export const viewsStore = new ViewsStore(cfg.viewsFile);
export const conflictRulingsStore = new ConflictRulingsStore(cfg.conflictRulingsFile);
export const eventsStore = new EventsStore(cfg.eventsFile);
export const reportRecordsLocation = storeRemotes?.aaLastGood ?? defaultReportRecordsFile();
export const identityRegistryStore = new IdentityRegistryStore(cfg.identityRegistryFile);
export const breakGlassTokenStore = new BreakGlassTokenStore(cfg.breakGlassFile);
export const outageQueueStore = new OutageQueueStore(cfg.outageQueueFile);

/**
 * #9608 (design #1253 section 6): the outage-queueing gate for action
 * writes whose Access verification failed. Queueing engages only on
 * the enforcer's 401 (verification failed — a 403 is a verified
 * identity contradicting the claim, not an outage), only from a
 * stack-local peer address, and only for eligible writes; anything
 * else returns null and the caller serves the enforcer's refusal
 * exactly as before. Throws when the queue/audit write itself fails —
 * callers map that to a 500 (fail closed: nothing was queued).
 */
export async function tryQueueOutageAction(
  action: NewAction,
  accessStatus: number,
  remoteAddress: string | undefined,
): Promise<{ record: OutageQueuedWrite; created: boolean } | null> {
  if (accessStatus !== 401) return null;
  if (!isStackLocalAddress(remoteAddress)) return null;
  if (!outageQueueEligible(action)) return null;
  return queueOutageWrite({ store: outageQueueStore, events: eventsStore, action });
}
export const freshnessStore = new FreshnessStore(cfg.freshnessFile);

/** #3976: the 5-minute anti-entropy reconciler over the freshness
 *  journal. Its one wired canonical reader is the fleet feed — the
 *  aggregator's own canonical source, fingerprinted exactly as the
 *  collector's notice identity (generatedAt + sha256 of the file).
 *  Its health view feeds /health + /api/health via healthContext. */
export const freshnessReconciler = new FreshnessReconciler(freshnessStore, [fleetFeedRevisionReader(cfg.feedFile)]);
export const activityStore = new IngestionStore(cfg.activityFile, storeRemotes?.activity);
export const githubHotListConsumer =
  cfg.source === "live" && cfg.projector.baseUrl && cfg.projector.apiToken
    ? new GithubHotListConsumer({
        baseUrl: cfg.projector.baseUrl,
        token: cfg.projector.apiToken,
        store: activityStore,
        events: eventsStore,
      })
    : null;

/** #10910: the outcome consumer rides the same sweep as the hot-list
 *  consumer, under its own projector checkpoint: it projects each fixed
 *  high-water into the append-only outcome-fact ledger, joining events
 *  to jobs/sessions/models only through the binding records. */
export const githubOutcomeConsumer =
  cfg.source === "live" && cfg.projector.baseUrl && cfg.projector.apiToken
    ? new GithubOutcomeConsumer({
        baseUrl: cfg.projector.baseUrl,
        token: cfg.projector.apiToken,
        store: new OutcomeFactStore(cfg.outcomeFile),
        bindings: new FileOutcomeBindingSource(cfg.outcomeBindingsFile),
      })
    : null;

export async function githubActivityStatus(): Promise<ActivitySourceStatus> {
  return (
    (await activityStore.sourceStatuses()).find((status) => status.source === "github") ?? unconfiguredGithubStatus()
  );
}

export function activitySourceStatusHtml(status: ActivitySourceStatus): string {
  const cursorDistance =
    status.checkpointCursor !== null && status.highWater !== null
      ? Math.max(0, status.highWater - status.checkpointCursor)
      : null;
  const cursorText = status.cursor === null ? "not checkpointed" : String(status.cursor);
  const checkpointText = status.checkpointCursor === null ? "not checkpointed" : String(status.checkpointCursor);
  const highWaterText = status.highWater === null ? "unknown" : String(status.highWater);
  const lagText = cursorDistance === null ? "unknown" : String(cursorDistance);
  const escapeHtml = (value: string) =>
    value.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
  const error = status.error ? `; detail: ${escapeHtml(status.error)}` : "";
  return (
    `<p>GitHub hot-list consumer: <strong>${status.state}</strong>; processed cursor ${cursorText}; ` +
    `acknowledged cursor ${checkpointText}; high-water ${highWaterText}; unacknowledged cursor distance ${lagText}; ` +
    `last successful sweep ${escapeHtml(status.lastSuccessAt ?? "never")}${error}. ` +
    `Current means caught up to the hot-list high-water, not complete GitHub history.</p>`
  );
}

/** #1739: the event log's fire-and-forget sink for source-side
 *  writers (orchestration publishes, registry merges). Never awaited,
 *  never throws: a failed event write loses the event, nothing else. */
export function recordStreamEvent(event: Parameters<typeof emitEvent>[1]): void {
  void emitEvent(eventsStore, event);
}

/** #3321: the activity-ingestion sink for the source-side writers
 *  (watchdog episodes, Kaneo board changes, GitHub hot-list rows). Dedupe + checkpoints
 *  live in the store. The returned promise resolves true only when
 *  every delivery is durable (ingested or already seen): the live
 *  source awaits it as the acknowledgment that lets it discard its
 *  Kaneo baseline (Codex P2, PR #5141) — failed deliveries retain
 *  their cards' baseline entries so the next poll retries the same
 *  stable delivery ids, while acknowledged cards advance. */
export async function recordActivity(
  deliveries: Parameters<typeof ingestActivity>[2],
): Promise<{ failed: number; failedIds: string[] }> {
  const result = await ingestActivity(activityStore, eventsStore, deliveries);
  return { failed: result.failed, failedIds: result.failedIds };
}

/** #3977: source freshness notices are not #1739 activity records.
 *  This store is owned by the aggregator process; the projector and
 *  collector reach it only through the loopback publisher listener. */
export async function recordFreshness(input: FreshnessInput): Promise<void> {
  await freshnessStore.publish(input);
}

/** #3321: the source-gap sink for the live source's polling gaps
 *  (Codex P2, PR #5141): a non-null `to` records (or extends) the
 *  open gap for the source; a null `to` is recovery — the open gap
 *  resolves as backfilled, since polling has resumed and the next
 *  diff lands against the preserved baseline. Never throws into the
 *  source's read path. */
export async function recordSourceGap(source: string, from: string, to: string | null): Promise<void> {
  try {
    if (to !== null) {
      await activityStore.recordGap(source, from, to);
      return;
    }
    for (const gap of await activityStore.gaps()) {
      if (gap.source === source && gap.status === "open") {
        // Recovery happened now, not at the last failed poll: extend
        // the gap window to the recovery moment as it resolves
        // (Codex P2, PR #5141).
        await activityStore.resolveGap(gap.id, "backfilled", new Date().toISOString());
      }
    }
  } catch (err) {
    console.error(
      `[activity] WARNING: source gap record failed for ${source}: ${err instanceof Error ? err.message : String(err)}`,
    );
  }
}

/** #3321: pins watchdog episodes to their first-observed `since`
 *  (Codex P2, PR #5141) — watchdogTierItems assigns `now` for
 *  several breach kinds, so without this the stream would gain a
 *  new "episode" on every attention render of one outage. */
export const watchdogEpisodes = new WatchdogEpisodeStabilizer();
export async function hasStreamEvent(kind: string, ref: string): Promise<boolean> {
  return eventsStore.has(kind, ref);
}

export function pickSource(): Source {
  // #1290: the live source reads the standing delegation default from
  // the actions store's preference records when it stamps the
  // needs-user fallbacks onto tier items.
  return cfg.source === "live"
    ? liveSource(cfg, {
        standingAuto: async () => effectiveStandingAuto(await actionsStore.all()).standingAuto,
        onEvent: recordStreamEvent,
        hasEvent: hasStreamEvent,
        onActivity: recordActivity,
        onFreshness: recordFreshness,
        onSourceGap: recordSourceGap,
        seenActivityIds: (source) => activityStore.seenIds(source),
        snapshotRemote: storeRemotes?.usage,
        projector: createPmCoreProjectorReader() ?? undefined,
        onLaptopUnreachable: (detail) => {
          fireOpenCodeWorkingHook("aggregator-tick", detail);
        },
      })
    : fixtureSource();
}

export const source = pickSource();

/** Lazy quota refresh (#1171): stale page/API loads ask the deployed
 *  gatherer unit to run again; single-flight + cooldown live in the module. */
export const quotaRefresher = createQuotaRefresher({ trigger: systemdTrigger() });

// #1279: prefer the latest-comment-per-ref read so a busy batch cannot
// truncate an item's most recent activity out of its brief.
export const briefCommentsFor = async (refs: RefKey[]): Promise<Comment[]> =>
  (source.latestComments ? source.latestComments(refs) : source.comments(refs)).catch(() => []);

/**
 * The source-conflict inbox (#1282 slice 1), assembled from the same facets
 * the rest of the dashboard reads. Read-only: detection collects what the
 * facets already report, and no render changes because of it.
 */
export async function assembleConflicts(): Promise<ConflictInbox> {
  const [pool, orchestration, usage, rulings] = await Promise.all([
    source.projects().catch(() => []),
    source.orchestration().catch(() => null),
    source.usage().catch(() => emptyUsageView()),
    conflictRulingsStore.all().catch(() => []),
  ]);
  return conflictInbox({ pool, orchestration, services: usage.services, rulings });
}

/** The identities half of the comment form options, behind a
 *  stale-while-revalidate cache (#5082, 2026-11-14). commentFormOpts
 *  is awaited on the request path of /program, /api/program.json and
 *  /board, and the registry read behind the identities merges the
 *  feed with a live map of the laptop OpenCode server — ~0.5 s when
 *  that server is healthy, but 6-10.5 s per call when it stalls under
 *  load (mapServerSessions' sequential fetches burn their 10 s
 *  aborts; a failed map was not cached, so every assembly paid the
 *  stall again). Pool-quota-steward run 381 measured the result:
 *  /program and /api/program.json 503 at ~10.5 s, /board 200 at
 *  10.5 s, everything else fast. Identities are comment-form picker
 *  metadata — they change when sessions start or end, on a minutes
 *  timescale — so they take the same contract as the page
 *  assemblies: the first load is awaited (and warmed at boot,
 *  below); an expired entry serves stale behind a single-flight
 *  background refresh, so no request ever waits on the registry
 *  merge again. The per-request githubRefs half is never cached. */
export const commentIdentitiesCache = new TodayCache<AuthorIdentity[]>(TODAY_CACHE_TTL_MS);
export async function commentIdentities(): Promise<AuthorIdentity[]> {
  return commentIdentitiesCache.get(async () => {
    const records = await source.registry().catch(() => []);
    return identitiesFromRecords(records);
  });
}

/** Author identities for the comment form: standing + registry sessions. */
export async function commentFormOpts(githubRefs: RefKey[] = []): Promise<CommentFormOpts> {
  return { githubRefs, identities: await commentIdentities() };
}

/** Resolve a discuss-page title for any object ref, best-effort. */
export async function discussTitle(objectRef: string): Promise<{ title: string; githubRefs: RefKey[] }> {
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

/** #1211: the fail-soft facet fallbacks — an empty view is the honest
 *  degraded form of a facet whose read threw (the renderers already
 *  banner absent data, e.g. buildAttention's tier-source-missing
 *  flag); a throwing facet must never 500 a whole page. */
export function emptyQuotaView(): QuotaView {
  return { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } };
}
export function emptyUsageView(): UsageView {
  return {
    services: [],
    uninstrumentedServices: [],
    payg: [],
    charges: [],
    attention: [],
    domains: null,
    subscriptions: [],
    history: {},
  };
}
