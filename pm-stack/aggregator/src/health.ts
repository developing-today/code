// Health assembly for /health + /api/health (#1147): per-service
// probe state, per-section feed ages, job runs, and this process's
// own page-render history — assembled from the same readers the rest
// of the dashboard trusts, so the page and the API can never disagree.
//
// Honesty rules (the #1110/#1139 lessons, applied to health itself):
// - Probes are real HTTP reads of the loopback targets in
//   src/topology.ts (from config where config has them), cached 60 s
//   so a human refreshing — or a watchdog polling — never fans out to
//   five services per hit. A failed probe renders as a failed probe,
//   with its error and its age; nothing here invents a state.
// - A service whose topology row is probe:"off" (the projector when
//   unconfigured) is never probed and never red.
// - Page health is an in-process render registry (server.ts records
//   each successful page render), never self-HTTP: a route nobody has
//   opened is labelled "not rendered since boot", not fake-green.
// - In fixtures mode there are no live probes to make: every service
//   reports "fixtures", which is also the degraded-state test fixture.
import type { PendingCommentCounts } from "./comments.ts";
import type { ConflictInbox } from "./conflicts.ts";
import type { Config } from "./config.ts";
import { unreportedCheckoutDrift, type CheckoutDrift } from "./checkout-drift.ts";
import {
  ageMsText,
  FEED_STALE_MS,
  GENERATED_AT_FUTURE_SKEW_MS,
  type FeedSectionStatus,
  type FleetFeed,
  type SectionDiagnosis,
} from "./feed.ts";
import type { GhItem, ProvenanceDetail, QuotaView, RefKey } from "./model.ts";
import type { PushHealthView } from "./freshness-reconcile.ts";
import { PAGES } from "./pages.ts";
import { buildServices, type ServiceDef } from "./topology.ts";
import {
  fetchRfaState,
  rfaHealthDown,
  rfaHealthFacet,
  type RfaHealthFacet,
  type RfaState,
} from "./sources/ready-for-agent.ts";

/** Orchestration cadence: hourly; next run is an expectation, not a promise. */
const ORCHESTRATION_CADENCE_MS = 3_600_000;

export const PROBE_CACHE_MS = 60_000;
export const PROBE_TIMEOUT_MS = 1_500;

/** The minimal fetch shape probes need — injectable for tests. */
export type ProbeFetch = (
  url: string,
  init: { signal: AbortSignal; redirect: "manual" },
) => Promise<{ status: number }>;

const defaultProbeFetch: ProbeFetch = async (url, init) => {
  const res = await fetch(url, init);
  return { status: res.status };
};

export interface ProbeOutcome {
  state: ServiceState;
  ok: boolean | null;
  detail: string;
  statusCode?: number;
  /** When the probe actually ran (a cached outcome keeps its own time). */
  probedAtMs?: number;
}

export type ServiceState = "up" | "down" | "stale" | "absent" | "invalid" | "unknown" | "off" | "self" | "fixtures";

export interface ProbeCache {
  entries: Map<string, { at: number; outcome: ProbeOutcome }>;
}

export function createProbeCache(): ProbeCache {
  return { entries: new Map() };
}

const sharedProbeCache = createProbeCache();

async function probeHttp(def: ServiceDef, fetcher: ProbeFetch, cache: ProbeCache, now: number): Promise<ProbeOutcome> {
  const hit = cache.entries.get(def.id);
  if (hit && now - hit.at < PROBE_CACHE_MS) return hit.outcome;
  const url = `${def.loopbackUrl}${def.healthPath ?? "/"}`;
  let outcome: ProbeOutcome;
  try {
    const res = await fetcher(url, { signal: AbortSignal.timeout(PROBE_TIMEOUT_MS), redirect: "manual" });
    const expected = def.expectStatuses ?? [200];
    outcome = expected.includes(res.status)
      ? { state: "up", ok: true, detail: `HTTP ${res.status}`, statusCode: res.status, probedAtMs: now }
      : {
          state: "down",
          ok: false,
          detail: `HTTP ${res.status} — expected ${expected.join(" or ")}`,
          statusCode: res.status,
          probedAtMs: now,
        };
  } catch (err) {
    const name = err instanceof Error ? err.name : "";
    const detail =
      name === "TimeoutError" || name === "AbortError"
        ? `probe timed out after ${PROBE_TIMEOUT_MS / 1000}s`
        : `probe failed: ${err instanceof Error ? err.message : String(err)}`;
    outcome = { state: "down", ok: false, detail, probedAtMs: now };
  }
  cache.entries.set(def.id, { at: now, outcome });
  return outcome;
}

export interface ServiceHealth {
  def: ServiceDef;
  state: ServiceState;
  ok: boolean | null;
  detail: string;
  statusCode?: number;
  probedAt?: string;
}

export interface FeedSectionHealth {
  key: string;
  present: boolean;
  /** `unknown` means an older collector did not publish outcome metadata. */
  status: FeedSectionStatus | "unknown";
  /** The timestamp the age is measured from (latest collection attempt when available). */
  generatedAt?: string;
  ageMs?: number;
  note?: string;
  /** Provider-reported total minus emitted entries; null means no comparable total. */
  behindBy: number | null;
  /** Latest successful source read, not the feed file's write time. */
  lastSyncAt: string | null;
  lagNote: string | null;
}

export interface QuotaGathererProviderHealth {
  providerId: string;
  serviceId: string;
  state: "ok" | "stale" | "error" | "lapsed";
  /** The latest successful sync, or null when the provider has never
   * synced: an errored entry's measuredAt is the failed attempt's time,
   * not a sync (Codex P2 on PR #5180). Retained (stale) and lapsed entries keep
   * their last successful measuredAt. */
  lastSyncAt: string | null;
  /** Quota providers do not currently publish a comparable expected row total. */
  behindBy: null;
  lagNote: string;
  credentialLabel?: string;
}

export interface PageHealth {
  route: string;
  title: string;
  lastRenderedAt: string | null;
  ok: boolean | null;
  ageMs: number | null;
}

/** Item resolution coverage (#1141): of the items one resolution pass
 *  served, how many came from live data (project pool + projector) and
 *  how many from fixtures. Untagged items (the source did not say) are
 *  counted in neither bucket — health never invents a source. */
export interface ItemsCoverage {
  resolvedLive: number;
  fixture: number;
  total: number;
  /** resolvedLive / total; null when nothing tagged was resolved. */
  ratio: number | null;
}

export function summarizeItemCoverage(items: Map<RefKey, GhItem>): ItemsCoverage {
  let resolvedLive = 0;
  let fixture = 0;
  for (const item of items.values()) {
    if (item.resolution === "fixture") fixture++;
    else if (item.resolution === "pool" || item.resolution === "projector") resolvedLive++;
  }
  const total = resolvedLive + fixture;
  return { resolvedLive, fixture, total, ratio: total ? resolvedLive / total : null };
}

/** The conflict-inbox count as the health surface carries it (#1282). */
export interface ConflictsHealth {
  total: number;
  byClass: Array<{ classId: string; label: string; count: number }>;
  /** Oldest live disagreement in whole days, or null when none is dated. */
  oldestAgeDays: number | null;
  generatedAt: string;
}

export interface HealthView {
  /** The aggregator process itself is up if it assembled this view. */
  status: "ok";
  mode: "live" | "fixtures";
  /** Deployed build marker (#1150): the commit install.sh deployed,
   *  carried into the process via PM_AGG_BUILD_VERSION in the service
   *  env file, or "unknown" for a process nobody deployed that way.
   *  install.sh polls /api/health after a restart and fails loudly
   *  when this does not match the tree it just deployed — a deploy
   *  that cannot prove it is live is a failed deploy. */
  version: string;
  generatedAt: string;
  bootAt: string;
  degraded: boolean;
  services: ServiceHealth[];
  feed: {
    generatedAt: string | null;
    fileMtime: string | null;
    ageMs: number | null;
    /** #1116 item 3: the collector checkout's drift vs origin/main,
     *  as reported in the feed; "unknown" with a reason when the feed
     *  carries no report — never a silently invented state. */
    checkout: CheckoutDrift;
    sections: FeedSectionHealth[];
  };
  quotaGatherer: {
    generatedAt: string;
    tickAt: string | null;
    ok: boolean;
    providers: QuotaGathererProviderHealth[];
  } | null;
  jobs: {
    orchestration: {
      state: SectionDiagnosis["state"] | "unknown";
      runId?: string;
      generatedAt?: string;
      ageMs?: number;
      nextExpectedAt?: string;
      failedChecks?: string[];
    };
    collector: { generatedAt: string | null; fileMtime: string | null; ageMs: number | null };
  };
  pages: PageHealth[];
  facets: ProvenanceDetail | null;
  /** #1141: resolved-live vs fixture counts from the priming pass. */
  itemsCoverage: ItemsCoverage | null;
  /** Dashboard comment loops (#1145): pending triage/posting counts,
   *  so an unworked comment queue is visible on /health. */
  comments: PendingCommentCounts;
  /** Source-conflict inbox (#1282 slice 1): how many live disagreements the
   *  facets report, so a growing conflict backlog is itself visible. Null
   *  when the caller did not assemble the inbox (older call sites/tests). */
  conflicts: ConflictsHealth | null;
  /** ready-for-agent harness facet (#1203): live GraphQL state —
   *  registered repositories, backend readiness, work-item counts —
   *  from src/sources/ready-for-agent.ts, not just the topology
   *  probe's HTTP 200. "down" with the error when the harness cannot
   *  be read; never an invented state. */
  rfa: RfaHealthFacet;
  /** #3976: push health — the freshness journal + 5-minute reconciler
   *  state (push up/degraded/down, last event, last successful
   *  reconcile, per-source age/lag, open/reset gaps), assembled by
   *  src/freshness-reconcile.ts. Separate from source freshness on
   *  purpose: a connected stream is not proof of fresh data, and a
   *  quiet source is not an outage. Null when the caller did not
   *  assemble it (fixtures mode, older call sites) — never invented. */
  push: PushHealthView | null;
}

// --- Render registry -------------------------------------------------------
// server.ts records every successful page render here. Module scope
// ≈ process scope: BOOT_AT is when this process started serving.

const BOOT_AT = Date.now();
const renderLog = new Map<string, { at: number; ok: boolean }>();

export function recordRender(route: string, ok: boolean, at = Date.now()): void {
  renderLog.set(route, { at, ok });
}

export function renderRegistrySize(): number {
  return renderLog.size;
}

/** The render registry as PageHealth rows (#1497: the /pages live
 * index renders exactly this — last-rendered + ok per manifest page,
 * straight from the same registry /health reports). */
export function pageRenderHealth(now = Date.now()): PageHealth[] {
  return pageHealth(now);
}

function pageHealth(now: number): PageHealth[] {
  return PAGES.map((p) => {
    const mark = renderLog.get(p.route);
    if (!mark) return { route: p.route, title: p.title, lastRenderedAt: null, ok: null, ageMs: null };
    return {
      route: p.route,
      title: p.title,
      lastRenderedAt: new Date(mark.at).toISOString(),
      ok: mark.ok,
      ageMs: Math.max(0, now - mark.at),
    };
  });
}

// --- Feed sections ---------------------------------------------------------

function feedSections(feed: FleetFeed | null, now: number): FeedSectionHealth[] {
  if (!feed) return [];
  const from = (generatedAt: string | undefined, present: boolean, note?: string): Partial<FeedSectionHealth> => {
    const ts = generatedAt ?? feed.generatedAt;
    const parsed = Date.parse(ts);
    // Section stamps get the same validation as the top-level one
    // (#2242, mirroring #1564): an unparseable stamp must not render
    // as ageMs NaN/null, and a stamp beyond the future-skew bound
    // must not render as ageMs 0 (maximally fresh) for as long as
    // the skew lasts. Either way the section reports absent with a
    // legible note instead of a fabricated age.
    if (!Number.isFinite(parsed)) {
      return { present: false, generatedAt: ts, note: `invalid generatedAt (${ts})` };
    }
    if (parsed - now > GENERATED_AT_FUTURE_SKEW_MS) {
      return {
        present: false,
        generatedAt: ts,
        note: `generatedAt (${ts}) is ${ageMsText(parsed - now)} in the future — never fresh`,
      };
    }
    return { present, generatedAt: ts, ageMs: Math.max(0, now - parsed), ...(note ? { note } : {}) };
  };
  const aa = feed.aa;
  const aaNote =
    aa && "status" in aa && aa.status === "absent"
      ? `absent form: ${"reason" in aa ? String(aa.reason) : "no reason given"}`
      : undefined;
  const section = (key: string, generatedAt: string | undefined, present: boolean, note?: string) => {
    const outcome = feed.meta?.sections[key];
    const outcomeNote =
      outcome?.status === "error"
        ? outcome.error
        : outcome?.status === "skipped"
          ? "intentionally skipped by collector"
          : undefined;
    const reportedTotal = outcome?.reportedTotal;
    const emittedEntries = outcome?.emittedEntries;
    const validReportedTotal =
      typeof reportedTotal === "number" && Number.isSafeInteger(reportedTotal) && reportedTotal >= 0
        ? reportedTotal
        : undefined;
    const validEmittedEntries =
      typeof emittedEntries === "number" && Number.isSafeInteger(emittedEntries) && emittedEntries >= 0
        ? emittedEntries
        : undefined;
    const behindBy =
      outcome?.status === "ok" &&
      validReportedTotal !== undefined &&
      validEmittedEntries !== undefined &&
      validReportedTotal >= validEmittedEntries
        ? validReportedTotal - validEmittedEntries
        : null;
    const lastSyncAt =
      outcome?.status === "ok"
        ? outcome.at
        : generatedAt && Number.isFinite(Date.parse(generatedAt))
          ? generatedAt
          : null;
    const lagNote =
      behindBy !== null
        ? null
        : outcome?.status === "error"
          ? "latest collection failed; no successful sync time is retained"
          : outcome?.status === "skipped"
            ? "collection was skipped; no sync time is reported"
            : validReportedTotal !== undefined && validEmittedEntries !== undefined && validReportedTotal < validEmittedEntries
              ? "emitted entry count exceeds the provider-reported total; lag is unknown"
              : "source does not report a comparable expected entry total";
    return {
      key,
      status: outcome?.status ?? "unknown",
      ...from(outcome?.at ?? generatedAt, present, outcomeNote ?? note),
      behindBy,
      lastSyncAt,
      lagNote,
    } as FeedSectionHealth;
  };
  return [
    section("quota", undefined, true),
    section("attention", undefined, true),
    section("orchestration", feed.orchestration?.generatedAt, feed.orchestration !== undefined),
    section("dns", feed.dns?.generatedAt, feed.dns !== undefined),
    section("registry", undefined, feed.registry !== undefined),
    section("projects", undefined, feed.projects !== undefined),
    section("coverage", undefined, feed.coverage !== undefined),
    section("saas", undefined, feed.saas !== undefined),
    section("payg", undefined, feed.payg !== undefined),
    section("charges", undefined, feed.charges !== undefined),
    section("chargesAttention", undefined, feed.chargesAttention !== undefined),
    section("subscriptions", undefined, feed.subscriptions !== undefined),
    section("snapshots", undefined, feed.snapshots !== undefined),
    section("aa", aa?.generatedAt, aa !== undefined, aaNote),
  ];
}

// --- Assembly ---------------------------------------------------------------

export interface HealthInputs {
  mode: "live" | "fixtures";
  /** Build marker override (tests); defaults to the deployed
   *  PM_AGG_BUILD_VERSION env value, else "unknown". */
  version?: string;
  cfg: Config;
  feed: FleetFeed | null;
  feedFileMtimeMs: number | null;
  quota?: QuotaView;
  diagnosis?: SectionDiagnosis;
  detail?: ProvenanceDetail;
  itemsCoverage?: ItemsCoverage | null;
  comments?: PendingCommentCounts;
  /** Source-conflict inbox (#1282 slice 1) — counted on the stack health
   *  surface so the backlog is visible where the stack's state already is. */
  conflicts?: ConflictInbox | null;
  /** #3976: the reconciler's current push-health view (server.ts
   *  supplies it in live mode); absent/null renders as unassembled. */
  pushHealth?: PushHealthView | null;
}

export interface HealthDeps {
  fetcher?: ProbeFetch;
  cache?: ProbeCache;
  now?: number;
  /** #1203 test seam: replace the live harness read. */
  rfaState?: () => Promise<RfaState>;
}

export function resolveBuildVersion(env: Record<string, string | undefined> = process.env): string {
  const v = env.PM_AGG_BUILD_VERSION?.trim();
  return v ? v : "unknown";
}

export async function buildHealth(inputs: HealthInputs, deps: HealthDeps = {}): Promise<HealthView> {
  const now = deps.now ?? Date.now();
  const fetcher = deps.fetcher ?? defaultProbeFetch;
  const cache = deps.cache ?? sharedProbeCache;
  const { cfg, feed } = inputs;
  const feedAgeMs = feed ? Math.max(0, now - Date.parse(feed.generatedAt)) : null;
  const feedFutureMs = feed ? Date.parse(feed.generatedAt) - now : null;
  const diagnosis = inputs.diagnosis;

  const services: ServiceHealth[] = [];
  const outcomes = new Map<string, ProbeOutcome>();
  for (const def of buildServices(cfg)) {
    let outcome: ProbeOutcome;
    if (inputs.mode === "fixtures") {
      outcome = { state: "fixtures", ok: null, detail: "fixtures mode — no live probes are made" };
    } else if (def.probe === "off") {
      outcome = { state: "off", ok: null, detail: "off by config — never probed, not an error" };
    } else if (def.probe === "self") {
      outcome = { state: "self", ok: true, detail: "this process — it assembled this view" };
    } else if (def.probe === "http") {
      outcome = await probeHttp(def, fetcher, cache, now);
    } else if (def.probe === "feed") {
      outcome =
        feed === null
          ? { state: "down", ok: false, detail: "no readable fleet feed" }
          : feedFutureMs !== null && feedFutureMs > GENERATED_AT_FUTURE_SKEW_MS
            ? {
                state: "stale",
                ok: false,
                detail: `feed generatedAt ${feed.generatedAt} is >60s in the future — never fresh`,
              }
            : feedAgeMs !== null && feedAgeMs >= FEED_STALE_MS
              ? {
                  state: "stale",
                  ok: false,
                  detail: `feed ${ageMsText(feedAgeMs)} — at or past the 5m freshness threshold`,
                }
              : { state: "up", ok: true, detail: `feed ${feedAgeMs !== null ? ageMsText(feedAgeMs) : ""}`.trim() };
    } else if (def.probe === "section") {
      if (!diagnosis) {
        outcome = { state: "unknown", ok: null, detail: "this source reports no section diagnosis" };
      } else if (diagnosis.state === "live") {
        outcome = {
          state: "up",
          ok: true,
          detail:
            `section ${diagnosis.runId ?? ""} · ${diagnosis.ageMs !== undefined ? ageMsText(diagnosis.ageMs) : ""}`.trim(),
        };
      } else if (diagnosis.state === "stale") {
        outcome = {
          state: "stale",
          ok: false,
          detail: `section ${diagnosis.runId ?? ""} is ${diagnosis.ageMs !== undefined ? ageMsText(diagnosis.ageMs) : "old"} — past the 90m stale threshold`,
        };
      } else if (diagnosis.state === "invalid") {
        outcome = {
          state: "invalid",
          ok: false,
          detail: `section present but rejected — failed checks: ${(diagnosis.failedChecks ?? []).join(", ") || "unknown"}`,
        };
      } else {
        outcome = {
          state: "absent",
          ok: false,
          detail: "no orchestration section in the feed the aggregator received",
        };
      }
    } else {
      // probe === "inferred": follows the named service's outcome.
      const parent = def.inferredFrom ? outcomes.get(def.inferredFrom) : undefined;
      outcome =
        parent === undefined
          ? { state: "unknown", ok: null, detail: "the service this is inferred from has no probe outcome" }
          : parent.state === "up"
            ? { state: "up", ok: true, detail: `inferred from ${def.inferredFrom} — container-internal, no host probe` }
            : {
                state: "unknown",
                ok: null,
                detail: `cannot infer — ${def.inferredFrom} is ${parent.state}, which proves nothing about this service`,
              };
    }
    outcomes.set(def.id, outcome);
    services.push({
      def,
      state: outcome.state,
      ok: outcome.ok,
      detail: outcome.detail,
      ...(outcome.statusCode !== undefined ? { statusCode: outcome.statusCode } : {}),
      ...(outcome.probedAtMs !== undefined ? { probedAt: new Date(outcome.probedAtMs).toISOString() } : {}),
    });
  }

  // #1203: the harness facet — a real GraphQL read of the dispatch
  // layer (cached 60 s inside fetchRfaState), reported down with its
  // error rather than hidden when the harness cannot be read.
  let rfa: RfaHealthFacet;
  if (inputs.mode === "fixtures") {
    rfa = { state: "fixtures", detail: "fixtures mode — no live harness read", repositories: [], backends: [] };
  } else {
    try {
      const state = deps.rfaState ? await deps.rfaState() : await fetchRfaState(cfg.readyForAgent.baseUrl);
      rfa = rfaHealthFacet(state);
    } catch (err) {
      rfa = rfaHealthDown(`harness read failed: ${err instanceof Error ? err.message : String(err)}`);
    }
  }

  const checkout = feed?.checkout ?? unreportedCheckoutDrift();
  const sectionHealth = feedSections(feed, now);
  const degraded =
    inputs.mode === "live" &&
    (services.some((s) => s.ok === false) ||
      (feedAgeMs !== null && feedAgeMs >= FEED_STALE_MS) ||
      checkout.state === "behind" ||
      // #3976: an unreadable freshness journal means push state
      // cannot be proven — that is degraded, never silently ok.
      (inputs.pushHealth != null && inputs.pushHealth.state === "down") ||
      // #1203: a harness whose GraphQL cannot be read (or whose backend
      // is not READY) is a degraded dependency, not an invisible one.
      rfa.state === "down" ||
      sectionHealth.some((section) => section.status === "error"));

  return {
    status: "ok",
    mode: inputs.mode,
    version: inputs.version ?? resolveBuildVersion(),
    generatedAt: new Date(now).toISOString(),
    bootAt: new Date(BOOT_AT).toISOString(),
    degraded,
    services,
    feed: {
      generatedAt: feed?.generatedAt ?? null,
      fileMtime: inputs.feedFileMtimeMs !== null ? new Date(inputs.feedFileMtimeMs).toISOString() : null,
      ageMs: feedAgeMs,
      checkout,
      sections: sectionHealth,
    },
    quotaGatherer: inputs.quota?.gatherer
      ? {
          generatedAt: inputs.quota.gatherer.generatedAt,
          tickAt: inputs.quota.gatherer.tickAt,
          ok: inputs.quota.gatherer.ok,
          providers: inputs.quota.gatherer.providers.map((provider) => ({
            providerId: provider.providerId,
            serviceId: provider.serviceId,
            state: provider.state,
            lastSyncAt: provider.state === "error" ? null : provider.measuredAt,
            behindBy: null,
            lagNote: "provider does not report a comparable expected entry total",
            ...(provider.credentialLabel ? { credentialLabel: provider.credentialLabel } : {}),
          })),
        }
      : null,
    jobs: {
      orchestration: {
        state: diagnosis?.state ?? "unknown",
        ...(diagnosis?.runId ? { runId: diagnosis.runId } : {}),
        ...(diagnosis?.generatedAt ? { generatedAt: diagnosis.generatedAt } : {}),
        ...(diagnosis?.ageMs !== undefined ? { ageMs: diagnosis.ageMs } : {}),
        ...(diagnosis?.generatedAt
          ? { nextExpectedAt: new Date(Date.parse(diagnosis.generatedAt) + ORCHESTRATION_CADENCE_MS).toISOString() }
          : {}),
        ...(diagnosis?.failedChecks ? { failedChecks: diagnosis.failedChecks } : {}),
      },
      collector: {
        generatedAt: feed?.generatedAt ?? null,
        fileMtime: inputs.feedFileMtimeMs !== null ? new Date(inputs.feedFileMtimeMs).toISOString() : null,
        ageMs: feedAgeMs,
      },
    },
    pages: pageHealth(now),
    rfa,
    push: inputs.pushHealth ?? null,
    facets: inputs.detail ?? null,
    itemsCoverage: inputs.itemsCoverage ?? null,
    comments: inputs.comments ?? { pendingMessages: 0, pendingGithub: 0, total: 0 },
    conflicts: inputs.conflicts
      ? {
          total: inputs.conflicts.total,
          byClass: inputs.conflicts.byClass,
          oldestAgeDays: inputs.conflicts.oldestAgeDays,
          generatedAt: inputs.conflicts.generatedAt,
        }
      : null,
  };
}
