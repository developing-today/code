// Staleness watchdog tier items (#1148): the surfacing half of the
// watchdog. The aggregator already DIAGNOSES its inputs — health.ts
// marks the fleet feed stale and reports the orchestration section's
// state, live.ts flips feed-backed facets stale over a dead feed, and
// topology probes report Kaneo/Grist/Pad failures — but before #1148
// all of that stayed on /health and in freshness chips: a dead input
// produced no work item in the fleet's queue, so it decayed silently.
//
// This module maps the current diagnoses to NEEDS ORCHESTRATOR tier
// items, exactly one per breached input, keyed stably per input
// (`watchdog:<input>`) so refreshes never duplicate one. The items are
// derived live from the current diagnosis on every attention render
// and are NEVER persisted: when an input recovers, its next derivation
// simply produces no item and the card clears itself. The watchdog
// never emits NEEDS DREW items — a dead input is fleet work, however
// long it persists; the persistence/escalation rules belong to the
// orchestrator's own alerting and are out of scope here.
//
// Thresholds (documented here, at the derivation site, per #1148):
// - fleet feed: age > 15 min is stale (FEED_STALE_MS, the same bound
//   health.ts and live.ts apply to feed-backed facets).
// - orchestration section: stale past 90 min (ORCHESTRATION_STALE_MS
//   in feed.ts, applied by diagnoseOrchestrationSection), and the
//   invalid / absent states breach immediately — there is no grace
//   period for a section that cannot be read at all.
// - registry facet: stale past 1 h (WATCHDOG_REGISTRY_STALE_MS). The
//   registry facet's age is the feed age of the records it merged
//   (live.ts provenanceDetail), so this is the slower, second alarm
//   behind the feed's own 15 min item: the feed item says collection
//   stopped; the registry item says session identity data itself is
//   now an hour old.
// - service probes: a failed probe (ok === false) for Kaneo, Grist,
//   or Pad breaches immediately; the probe cache (60 s, health.ts)
//   is the debounce.
import { ageMsText, GENERATED_AT_FUTURE_SKEW_MS, ORCHESTRATION_STALE_MS, type SectionDiagnosis } from "./feed.ts";
import type { TierItem } from "./model.ts";

/** Feed freshness bound — mirrors health.ts / live.ts FEED_STALE_MS. */
export const WATCHDOG_FEED_STALE_MS = 15 * 60_000;
/** Orchestration staleness bound — the diagnosis applies it; named
 *  here so the watchdog's contract reads in one place. */
export const WATCHDOG_ORCHESTRATION_STALE_MS = ORCHESTRATION_STALE_MS;
/** Registry facet staleness bound (#1148): 1 h. */
export const WATCHDOG_REGISTRY_STALE_MS = 60 * 60_000;
/** The topology services whose probe failure is a watchdog breach. */
export const WATCHDOG_PROBE_SERVICE_IDS: readonly string[] = ["kaneo", "grist", "pad"];

/** One service's probe outcome, in the shape health.ts assembles. */
export interface WatchdogProbe {
  /** Topology service id ("kaneo" | "grist" | "pad" | ...). */
  id: string;
  /** Human service name, for the item title. */
  name: string;
  /** false = the probe ran and failed; null = no verdict (off,
   *  fixtures, inferred) and never a breach. */
  ok: boolean | null;
  detail: string;
  /** When the probe actually ran (cached outcomes keep their time),
   *  so a card's `since` is stable across refreshes. */
  probedAt?: string;
}

/** Everything the watchdog derives from, assembled by the caller from
 *  the same readers the rest of the dashboard trusts (health.ts's
 *  feed ages + probe outcomes, the live source's orchestration
 *  diagnosis and registry facet age). */
export interface WatchdogInputs {
  now: number;
  /** Age of the fleet feed file's generatedAt, or null when there is
   *  no readable feed at all. */
  feedAgeMs: number | null;
  feedGeneratedAt?: string | null;
  /** The orchestration section diagnosis (#1139), when the source
   *  provides one. */
  orchestration?: SectionDiagnosis;
  /** The registry facet's age (live.ts provenanceDetail), when known. */
  registryAgeMs?: number;
  /** Probe outcomes for the topology services; only the
   *  WATCHDOG_PROBE_SERVICE_IDS rows are considered. */
  probes?: WatchdogProbe[];
}

const WATCHDOG_REF = "developing-today/code#1148" as TierItem["ref"];

function item(input: string, title: string, state: string, reason: string, sinceMs: number, now: number, action: string): TierItem {
  return {
    id: `watchdog:${input}`,
    kind: "gap",
    title,
    source: "watchdog",
    state,
    reason,
    since: new Date(sinceMs).toISOString(),
    lastSeenAt: new Date(now).toISOString(),
    ref: WATCHDOG_REF,
    recommendedAction: action,
  };
}

/**
 * Derive the current watchdog tier items from the current diagnoses.
 * Pure: the same inputs always produce the same items (ids, titles,
 * and — where the input carries its own timestamp — the same
 * `since`), so repeated derivations never duplicate or churn a card.
 * Returns items in input order: feed, orchestration, registry, then
 * probes in WATCHDOG_PROBE_SERVICE_IDS order.
 */
export function watchdogTierItems(inputs: WatchdogInputs): TierItem[] {
  const { now } = inputs;
  const out = new Map<string, TierItem>();
  const put = (it: TierItem) => {
    if (!out.has(it.id)) out.set(it.id, it);
  };

  // Fleet feed: future-dated (buildHealth's own failure diagnosis -
  // a generatedAt >60s ahead is never fresh, and its clamped age of 0
  // must not suppress this alarm until the future time + 15m),
  // stale past 15 min, or absent entirely.
  const feedFutureMs = inputs.feedGeneratedAt ? Date.parse(inputs.feedGeneratedAt) - now : NaN;
  if (!Number.isNaN(feedFutureMs) && feedFutureMs > GENERATED_AT_FUTURE_SKEW_MS) {
    put(
      item(
        "fleet-feed",
        `Watchdog: fleet feed is future-dated - generatedAt is ${ageMsText(feedFutureMs)} ahead of the aggregator clock`,
        "watchdog: feed future-dated",
        "buildHealth diagnoses a feed generatedAt more than 60s in the future as a failed, never-fresh feed (#1148 watchdog); the clamped age of 0 must not silence the watchdog for the whole skew.",
        now,
        now,
        "Check the fleet-feed collector's clock and the feed file it writes; the card clears itself when a feed with a sane generatedAt lands.",
      ),
    );
  } else if (inputs.feedAgeMs === null) {
    put(
      item(
        "fleet-feed",
        "Watchdog: fleet feed is unreadable — no feed to derive from",
        "watchdog: feed absent",
        "The fleet-feed collector's output cannot be read, so every feed-backed facet is running on its last good data or fixtures. Diagnosed by the aggregator's health assembly (#1148 watchdog).",
        now,
        now,
        "Check the fleet-feed collector (laptop user timer fleet-feed-collector) and the feed file it writes; the card clears itself when a readable feed lands.",
      ),
    );
  } else if (inputs.feedAgeMs > WATCHDOG_FEED_STALE_MS) {
    const generatedMs = inputs.feedGeneratedAt ? Date.parse(inputs.feedGeneratedAt) : now - inputs.feedAgeMs;
    put(
      item(
        "fleet-feed",
        `Watchdog: fleet feed is stale — last collected ${ageMsText(inputs.feedAgeMs)} ago`,
        "watchdog: feed stale",
        `Feed age ${ageMsText(inputs.feedAgeMs)} is past the 15m freshness threshold (#1148 watchdog); feed-backed facets are flipping stale over the dead feed.`,
        (Number.isNaN(generatedMs) ? now - inputs.feedAgeMs : generatedMs) + WATCHDOG_FEED_STALE_MS,
        now,
        "Check the fleet-feed collector (laptop user timer fleet-feed-collector, ~5 min cadence); the card clears itself when the feed is fresh again.",
      ),
    );
  }

  // Orchestration section: stale (>90m) / invalid / absent.
  const orch = inputs.orchestration;
  if (orch) {
    if (orch.state === "stale") {
      const generatedMs = orch.generatedAt ? Date.parse(orch.generatedAt) : now - (orch.ageMs ?? 0);
      put(
        item(
          "orchestration",
          `Watchdog: orchestration section is stale — last run ${orch.ageMs !== undefined ? ageMsText(orch.ageMs) : "over 90m"} old`,
          "watchdog: orchestration stale",
          `The orchestration section is ${orch.ageMs !== undefined ? ageMsText(orch.ageMs) : "past 90m"} old — past the 90m stale threshold (#1148 watchdog), so the published tiers are aging in place.`,
          (Number.isNaN(generatedMs) ? now : generatedMs) + WATCHDOG_ORCHESTRATION_STALE_MS,
          now,
          "Check the hourly orchestration-pass job; the card clears itself when a fresh section publishes.",
        ),
      );
    } else if (orch.state === "invalid") {
      put(
        item(
          "orchestration",
          "Watchdog: orchestration section is invalid — present but rejected",
          "watchdog: orchestration invalid",
          `The orchestration section failed the aggregator's structural checks (${(orch.failedChecks ?? []).join(", ") || "unknown"}), so no tiers are being served from it (#1148 watchdog).`,
          now,
          now,
          "Inspect the latest orchestration-pass output against the section gate in feed.ts; the card clears itself when a valid section publishes.",
        ),
      );
    } else if (orch.state === "absent") {
      put(
        item(
          "orchestration",
          "Watchdog: orchestration section is absent — no section in the feed",
          "watchdog: orchestration absent",
          "The feed the aggregator received carries no orchestration section, so there are no published tiers at all (#1148 watchdog).",
          now,
          now,
          "Check whether the orchestration-pass job has run and whether the collector is carrying its section; the card clears itself when a section appears.",
        ),
      );
    }
  }

  // Registry facet: stale past 1 h.
  if (inputs.registryAgeMs !== undefined && inputs.registryAgeMs > WATCHDOG_REGISTRY_STALE_MS) {
    const generatedMs = inputs.feedGeneratedAt ? Date.parse(inputs.feedGeneratedAt) : now - inputs.registryAgeMs;
    put(
      item(
        "registry",
        `Watchdog: registry facet is stale — session records ${ageMsText(inputs.registryAgeMs)} old`,
        "watchdog: registry stale",
        `The identity registry's merged session records are ${ageMsText(inputs.registryAgeMs)} old — past the 1h registry threshold (#1148 watchdog) — so session identity/coverage data is decaying behind the feed's own 15m alarm.`,
        (Number.isNaN(generatedMs) ? now - inputs.registryAgeMs : generatedMs) + WATCHDOG_REGISTRY_STALE_MS,
        now,
        "Check the registry inputs (session launch records in the fleet feed); the card clears itself when fresh records merge.",
      ),
    );
  }

  // Service probes: a failed Kaneo/Grist/Pad probe breaches at once.
  for (const id of WATCHDOG_PROBE_SERVICE_IDS) {
    const probe = (inputs.probes ?? []).find((p) => p.id === id);
    if (!probe || probe.ok !== false) continue;
    const probedMs = probe.probedAt ? Date.parse(probe.probedAt) : NaN;
    put(
      item(
        `probe-${id}`,
        `Watchdog: ${probe.name} probe is failing — ${probe.detail}`,
        "watchdog: probe failing",
        `The aggregator's topology probe for ${probe.name} reports a failure (${probe.detail}); /health has the full probe state (#1148 watchdog).`,
        Number.isNaN(probedMs) ? now : probedMs,
        now,
        `Check the ${probe.name} service on the laptop (see the topology table on /health for its unit/container); the card clears itself when the probe passes.`,
      ),
    );
  }

  return [...out.values()];
}
