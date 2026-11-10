// Pure forecast derivations for epic ETA bands and item stall risk
// (#3982, slice 2 of the forecasting design,
// doc/2026-11-01T21-07-00Z_design_pm-stack_forecasting.md §1, §3, §4).
//
// Everything here is a pure function over the versioned item-history
// cutoffs captured by #3981 (src/item-history.ts): no fetching, no
// store, no second history. The honesty rules of the design are the
// contract of this module:
//
// - The throughput unit is one direct native child ISSUE closure.
//   Merged PRs are context, never units, so an issue and its PR can
//   never double-count; a reopened child is not currently complete.
// - Only complete observed weekly buckets take part, and zero-
//   completion weeks count: they are real observations, not gaps.
//   Incomplete/unavailable cutoffs are excluded, never read as zero.
// - A numeric ETA requires at least 3 completed units and 4 complete
//   observed weeks in scope. Below that, the result is either a
//   clearly named borrowed baseline (low confidence) or
//   "not_enough_history" with NO dates — never an invented date.
//
// - The estimator (design §1) resamples the complete weekly
//   completion buckets — zero weeks included — with replacement
//   under a seeded PRNG, estimating the distribution of
//   weeks-to-complete for the remaining units; P50 and the
//   conservative P80 are percentiles of that distribution. The seed
//   derives from the buckets and the remaining-unit count, so the
//   same inputs always produce the same forecast. A trial that does
//   not finish within the per-trial week cap is a CENSORED
//   observation, never a completion at the cap: a percentile whose
//   rank falls on a censored trial lies beyond the simulation
//   horizon and returns no date, with a limitation saying so
//   (#6386). A scope whose
//   total observed pace is zero still returns "no_measurable_pace":
//   resampling zero buckets can only ever draw zero.
// - Effort stays explicit: remaining work is reported per effort
//   label, unsized work is counted as unsized, and `l` (an open-ended
//   floor in weights.ts) is flagged open-ended. No size is inferred.
// - Stall risk is a versioned, explainable 0-100 rule score with
//   reason codes and timestamps. Blocker age is a first-observed
//   LOWER BOUND (item-history.ts), labelled as such. Signals a source
//   does not supply stay unknown: they contribute nothing and reduce
//   evidence coverage. Score >= 60 is informational "at risk" only —
//   by design (§3) it never creates a Needs-you escalation by itself.

import type { ItemHistoryCutoff } from "./item-history.ts";
import { blockerFirstObserved, weeklyTransitions } from "./item-history.ts";
import type { RefKey } from "./model.ts";

// forecast/v2 (#5274): the ETA pace derivation is the design §1
// resampling estimator, replacing v1's median/lower-quartile pace.
// Outputs for the same inputs change materially (bursty scopes with
// a zero median bucket now forecast instead of returning
// no_measurable_pace), so the model version bumps.
// forecast/v3 (#6386): capped resampling trials are censored
// observations, not completions at the cap. Outputs change
// materially for horizon-exceeding scopes (a percentile beyond the
// simulation horizon now returns no date instead of a date at the
// cap, which understated the true completion), so the version bumps
// again.
export const FORECAST_MODEL_VERSION = "forecast/v3";
export const STALL_MODEL_VERSION = "stall-risk/v1";

/** At/above this score an item is surfaced as "at risk" (§3). */
export const STALL_AT_RISK_THRESHOLD = 60;

const DAY_MS = 86_400_000;

/** True when instant `a` is at or before instant `b`, compared as
 * instants, not strings: the same instant can be written with or
 * without fractional seconds or with a timezone offset, and string
 * order disagrees with instant order across such representations
 * (e.g. "2026-11-02T00:00:00Z" sorts AFTER "2026-11-02T00:00:00.000Z"
 * as a string but is the same instant). Unparseable values fall back
 * to string order so behaviour stays deterministic. */
function atOrBefore(a: string, b: string): boolean {
  const x = Date.parse(a);
  const y = Date.parse(b);
  return Number.isNaN(x) || Number.isNaN(y) ? a <= b : x <= y;
}

/** Instant order for cutoff sorting, string order as tie-break. */
function instantOrder(a: string, b: string): number {
  const x = Date.parse(a);
  const y = Date.parse(b);
  if (Number.isNaN(x) || Number.isNaN(y) || x === y) return a.localeCompare(b);
  return x - y;
}

function completeCutoffs(cutoffs: ItemHistoryCutoff[]): ItemHistoryCutoff[] {
  return [...cutoffs]
    .filter((c) => c.coverage.complete && !c.coverage.unavailable)
    .sort((a, b) => instantOrder(a.cutoffAt, b.cutoffAt));
}

function addDays(iso: string, days: number): string {
  return new Date(Date.parse(iso) + days * DAY_MS).toISOString();
}

/** ETA date a whole number of weeks after `asOf`, or null when the
 * horizon is not representable as a Date (an absurdly distant horizon
 * would otherwise make toISOString throw a RangeError). */
function etaDateForWeeks(asOf: string, weeks: number): string | null {
  const ms = Date.parse(asOf) + weeks * 7 * DAY_MS;
  if (!Number.isFinite(ms) || Math.abs(ms) > 8.64e15) return null;
  return addDays(asOf, weeks * 7);
}

/** mulberry32: a small seeded PRNG. The estimator's seed derives
 * from its inputs, so resampling is a pure function of them — the
 * same buckets and remaining-unit count always yield the same
 * distribution, and therefore the same P50/P80 (determinism is part
 * of this module's contract, asserted by the forecast tests). */
function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** FNV-1a hash of the estimator inputs, used as the resampling seed. */
function estimatorSeed(buckets: number[], remainingUnits: number): number {
  const text = `${buckets.join(",")}|${remainingUnits}`;
  let h = 0x811c9dc5;
  for (let i = 0; i < text.length; i++) {
    h ^= text.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

const RESAMPLE_TRIALS = 4096;
/** Per-trial week cap: with any positive bucket a trial finishes
 * almost surely long before this; the cap only bounds pathological
 * inputs (a huge remaining count over near-zero pace) so the pure
 * function always terminates. A trial that reaches the cap
 * unfinished is CENSORED — it is not a completion at the cap.
 * (Before #6386 a capped trial read as a completion at that many
 * weeks; the old comment here called that "conservative", which is
 * backwards for a trial whose true completion exceeds the cap:
 * recording the cap pulls the percentile EARLIER than the true
 * completion and publishes an optimistic date.) */
const RESAMPLE_MAX_WEEKS = 100_000;

/**
 * The design §1 resampling estimator: draw complete weekly
 * completion buckets (zero weeks included — they are observations,
 * not gaps) with replacement until the drawn completions cover the
 * remaining units; the number of drawn weeks is one sample of the
 * weeks-to-complete distribution. Returns the sorted samples, with
 * a trial that does not finish within RESAMPLE_MAX_WEEKS as a
 * censored observation (null, sorted last) — never as a completion
 * at the cap (#6386).
 * Precondition: the buckets' total is positive (the caller returns
 * no_measurable_pace otherwise — resampling all-zero buckets can
 * only draw zero and no trial would ever finish).
 */
function resampledWeeksToComplete(buckets: number[], remainingUnits: number): (number | null)[] {
  const rng = mulberry32(estimatorSeed(buckets, remainingUnits));
  const maxBucket = Math.max(...buckets);
  const samples: (number | null)[] = [];
  for (let trial = 0; trial < RESAMPLE_TRIALS; trial++) {
    let completed = 0;
    let weeks = 0;
    while (completed < remainingUnits && weeks < RESAMPLE_MAX_WEEKS) {
      // Exact early censoring: when the units still remaining exceed
      // what drawing the largest bucket every remaining week could
      // cover, this trial cannot finish within the cap — it is
      // censored now, and no further draws can change that.
      if (remainingUnits - completed > (RESAMPLE_MAX_WEEKS - weeks) * maxBucket) break;
      completed += buckets[Math.floor(rng() * buckets.length)]!;
      weeks++;
    }
    samples.push(completed >= remainingUnits ? weeks : null);
  }
  return samples.sort((a, b) => (a ?? Number.POSITIVE_INFINITY) - (b ?? Number.POSITIVE_INFINITY));
}

/** Nearest-rank percentile of an ascending-sorted sample, or null
 * when the rank falls on a censored sample: that percentile lies
 * beyond the simulation horizon, so no weeks value is defensible. */
function percentile(sortedXs: (number | null)[], q: number): number | null {
  const idx = Math.min(sortedXs.length - 1, Math.max(0, Math.ceil(q * sortedXs.length) - 1));
  return sortedXs[idx] ?? null;
}

export type ForecastConfidence = "high" | "medium" | "low" | "unavailable";

export type EtaStatus = "forecast" | "borrowed_baseline" | "not_enough_history" | "no_measurable_pace";

export interface BorrowedBaseline {
  /** The comparable scope the rate is borrowed from, named verbatim
   * in the result (e.g. a project or portfolio ref). Never silent. */
  scopeRef: string;
  /** Completions per complete observed week in that scope. */
  weeklyCompletions: number[];
}

export interface EpicEtaInput {
  /** The epic/project subject the ETA describes. */
  subjectRef: RefKey;
  /** The full stored history (any order; incomplete cutoffs are
   * filtered out here). */
  cutoffs: ItemHistoryCutoff[];
  /** Direct native children of the subject. When omitted, children
   * are derived from the latest complete cutoff: issue items whose
   * parentRef is the subject (epicRef is ancestor membership and
   * would also match grandchildren, so it is not used). */
  childRefs?: RefKey[];
  /** Optional comparable-scope rate for the low-evidence case. */
  borrowedBaseline?: BorrowedBaseline;
  /** Forecast instant; defaults to the latest complete cutoff. */
  asOf?: string;
}

export interface EpicEta {
  subjectRef: RefKey;
  asOf: string;
  modelVersion: string;
  status: EtaStatus;
  /** Scope actually used for the rate: the subject, or the named
   * borrowed baseline's scope. */
  baselineSource: string;
  inputWindow: {
    primaryWeeks: number;
    comparisonWeeks: number;
    completeWeeks: number;
    from: string | null;
    to: string | null;
  };
  inputCoverage: {
    completeWeeks: number;
    totalCutoffs: number;
    unavailableCutoffs: number;
  };
  remainingUnits: number;
  completedUnits: number;
  sample: {
    completionsPrimary: number;
    completionsComparison: number;
  };
  /** Remaining work by effort label; `unsized` counts children with
   * no effort label and `openEnded` counts the open-ended floor
   * sizes (`l`, `xl`) — explicit, never folded into the unit count. */
  remainingEffort: {
    byEffort: Record<string, number>;
    unsized: number;
    openEnded: number;
  };
  /** ISO dates, or null when no defensible date exists. */
  p50: string | null;
  p80: string | null;
  confidence: ForecastConfidence;
  limitations: string[];
}

/**
 * Derive the ETA band for one epic/project subject over the item
 * history. Deterministic: the same cutoffs always produce the same
 * result, and every number in it names its window and sample.
 */
export function deriveEpicEta(input: EpicEtaInput): EpicEta {
  // Rolling-origin honesty (design \u00a75): a forecast "as of" an instant
  // uses only cutoffs observed at or before that instant \u2014 later
  // cutoffs are future outcomes and must not leak into scope,
  // transitions, or coverage.
  const scopedCutoffs = input.asOf ? input.cutoffs.filter((c) => atOrBefore(c.cutoffAt, input.asOf!)) : input.cutoffs;
  const complete = completeCutoffs(scopedCutoffs);
  const latest = complete[complete.length - 1];
  const asOf = input.asOf ?? latest?.cutoffAt ?? new Date(0).toISOString();
  const limitations: string[] = [];

  const base = {
    subjectRef: input.subjectRef,
    asOf,
    modelVersion: FORECAST_MODEL_VERSION,
    inputWindow: {
      primaryWeeks: 0,
      comparisonWeeks: 0,
      completeWeeks: complete.length,
      from: complete[0]?.cutoffAt ?? null,
      to: latest?.cutoffAt ?? null,
    },
    inputCoverage: {
      completeWeeks: complete.length,
      totalCutoffs: scopedCutoffs.length,
      unavailableCutoffs: scopedCutoffs.filter((c) => c.coverage.unavailable).length,
    },
  };

  if (!latest) {
    return {
      ...base,
      status: "not_enough_history",
      baselineSource: input.subjectRef,
      remainingUnits: 0,
      completedUnits: 0,
      sample: { completionsPrimary: 0, completionsComparison: 0 },
      remainingEffort: { byEffort: {}, unsized: 0, openEnded: 0 },
      p50: null,
      p80: null,
      confidence: "unavailable",
      limitations: ["no complete observed cutoffs in the item history"],
    };
  }

  // Scope: direct native child issues at the latest complete cutoff.
  // Direct means parentRef === subject: epicRef is ANCESTOR membership
  // (projects.ts walks the whole parent chain), so counting epicRef
  // would count grandchildren as independent units.
  const childRefs =
    input.childRefs ??
    latest.items.filter((i) => i.kind !== "pr" && i.parentRef === input.subjectRef).map((i) => i.ref);
  const childSet = new Set(childRefs);
  const children = latest.items.filter((i) => childSet.has(i.ref) && i.kind !== "pr");
  // Explicitly requested children absent from the latest complete
  // cutoff are unknown, not complete: a complete cutoff establishes
  // coverage of the fetched pool, not the presence of every requested
  // ref. They surface as a scope limitation below and block the
  // nothing-left branch.
  const missingRefs =
    input.childRefs === undefined ? [] : childRefs.filter((ref) => !children.some((i) => i.ref === ref));
  const completedUnits = children.filter((i) => i.state !== "open").length;
  // A reopened child is open at the latest cutoff: it is remaining
  // work, not a completion, whatever earlier weeks observed.
  const remaining = children.filter((i) => i.state === "open");
  const remainingUnits = remaining.length;

  const byEffort: Record<string, number> = {};
  let unsized = 0;
  let openEnded = 0;
  for (const item of remaining) {
    if (item.effort === null) unsized++;
    else {
      byEffort[item.effort] = (byEffort[item.effort] ?? 0) + 1;
      if (item.effort === "l" || item.effort === "xl") openEnded++;
    }
  }
  if (missingRefs.length > 0) {
    limitations.push(
      `${missingRefs.length} explicitly requested child(ren) absent from the latest complete cutoff; state unknown: ${missingRefs.join(", ")}`,
    );
  }
  if (unsized > 0) limitations.push(`${unsized} remaining unit(s) are unsized; no size was inferred`);
  if (openEnded > 0) {
    limitations.push(`${openEnded} remaining unit(s) carry an open-ended effort floor (l/xl)`);
  }

  // Throughput series restricted to this scope's children. Weeks are
  // the complete observed buckets from item-history (zero weeks
  // included by construction).
  const weeks = weeklyTransitions(scopedCutoffs)
    .map((w) => ({
      cutoffAt: w.cutoffAt,
      completed: w.completedRefs.filter((r) => childSet.has(r)).length,
    }))
    .filter((w) => complete.some((c) => c.cutoffAt === w.cutoffAt));
  const primary = weeks.slice(-4).map((w) => w.completed);
  const comparison = weeks.slice(-8).map((w) => w.completed);
  const completionsPrimary = primary.reduce((a, b) => a + b, 0);
  const completionsComparison = comparison.reduce((a, b) => a + b, 0);

  const result: EpicEta = {
    ...base,
    status: "forecast",
    baselineSource: input.subjectRef,
    inputWindow: {
      ...base.inputWindow,
      primaryWeeks: primary.length,
      comparisonWeeks: comparison.length,
    },
    remainingUnits,
    completedUnits,
    sample: { completionsPrimary, completionsComparison },
    remainingEffort: { byEffort, unsized, openEnded },
    p50: null,
    p80: null,
    confidence: "unavailable",
    limitations,
  };

  if (remainingUnits === 0 && missingRefs.length > 0) {
    // Every observed child is complete, but requested children are
    // unobserved: the subject cannot be declared complete while their
    // state is unknown, and no completion date is defensible.
    result.status = "not_enough_history";
    limitations.push(
      "explicitly requested child(ren) are unobserved; the subject is not declared complete while their state is unknown",
    );
    return result;
  }

  if (remainingUnits === 0 && completedUnits > 0) {
    // Nothing left: the ETA is the as-of instant itself.
    result.p50 = asOf;
    result.p80 = asOf;
    result.confidence = completionsComparison >= 3 && comparison.length >= 4 ? "medium" : "low";
    return result;
  }

  const enoughLocal = completionsPrimary >= 3 && primary.length >= 4;
  if (!enoughLocal && primary.length >= 4 && completionsPrimary === 0) {
    // A full primary window with zero completions is a measured
    // pace — of zero. That is "no measurable pace" (§1), not a
    // missing sample: no date, and a borrowed rate would hide it.
    result.status = "no_measurable_pace";
    limitations.push("zero completions across the 4 complete primary weeks; no date is returned");
    return result;
  }
  let paceSeries: number[];
  if (enoughLocal) {
    paceSeries = primary;
  } else if (input.borrowedBaseline && input.borrowedBaseline.weeklyCompletions.length >= 4) {
    paceSeries = input.borrowedBaseline.weeklyCompletions.slice(-4);
    result.status = "borrowed_baseline";
    result.baselineSource = input.borrowedBaseline.scopeRef;
    result.confidence = "low";
    limitations.push(
      `local evidence is below the numeric-forecast floor (3 completions / 4 complete weeks); rate borrowed from ${input.borrowedBaseline.scopeRef}`,
    );
  } else {
    result.status = "not_enough_history";
    limitations.push(
      "fewer than 3 completions or 4 complete weeks in scope and no comparable borrowed baseline: no ETA date is defensible",
    );
    return result;
  }

  const totalPace = paceSeries.reduce((a, b) => a + b, 0);
  if (totalPace <= 0) {
    // A truly zero total pace is "no measurable pace" (§1), exactly
    // as before: resampling all-zero buckets can only draw zero, so
    // no trial would ever cover the remaining units. A zero MEDIAN
    // with positive total pace (bursty buckets like [0, 0, 0, 4]) is
    // different — the resampling estimator below forecasts it,
    // where v1's median pace returned no_measurable_pace (#5274).
    result.status = "no_measurable_pace";
    limitations.push("observed weekly pace is zero in the primary window; no date is returned");
    return result;
  }
  // Whole weeks only: a partial week is not a completion date. P50 is
  // the median weeks-to-complete; P80 is the conservative (slower)
  // tail of the same distribution, so P50 <= P80 by construction.
  const weeksToComplete = resampledWeeksToComplete(paceSeries, remainingUnits);
  const censoredTrials = weeksToComplete.filter((w) => w === null).length;
  const p50Weeks = percentile(weeksToComplete, 0.5);
  const p80Weeks = percentile(weeksToComplete, 0.8);
  result.p50 = p50Weeks === null ? null : etaDateForWeeks(asOf, p50Weeks);
  if (p50Weeks === null) {
    limitations.push(
      `P50 exceeds the simulation horizon: ${censoredTrials} of ${RESAMPLE_TRIALS} resampled trials did not complete within the ${RESAMPLE_MAX_WEEKS}-week cap (censored observations, not completions), so no P50 date is returned`,
    );
  } else if (result.p50 === null) {
    limitations.push("P50 horizon exceeds the representable date range; no P50 date is returned");
  }
  result.p80 = p80Weeks === null ? null : etaDateForWeeks(asOf, p80Weeks);
  if (p80Weeks === null) {
    limitations.push(
      `P80 exceeds the simulation horizon: ${censoredTrials} of ${RESAMPLE_TRIALS} resampled trials did not complete within the ${RESAMPLE_MAX_WEEKS}-week cap (censored observations, not completions), so no P80 date is returned`,
    );
  } else if (result.p80 === null) {
    limitations.push("P80 horizon exceeds the representable date range; no P80 date is returned");
  }

  if (result.status === "forecast") {
    const comparisonRate = comparison.length > 0 ? completionsComparison / comparison.length : 0;
    const primaryRate = completionsPrimary / primary.length;
    const stable = comparisonRate > 0 && Math.abs(primaryRate - comparisonRate) / comparisonRate <= 0.25;
    result.confidence = completionsComparison >= 12 && comparison.length >= 8 && stable ? "high" : "medium";
    if (!stable) limitations.push("4-week and 8-week rates differ materially; confidence capped");
  }
  if (p50Weeks === null && p80Weeks === null) {
    // Both percentiles lie beyond the simulation horizon: no date
    // was returned at all, so no confidence attaches to one.
    result.confidence = "unavailable";
  }
  return result;
}

// ------------------------------------------------------------------
// Stall risk (§3)

export interface QueueSignals {
  /** Expected heartbeat intervals missed by the live claim. */
  missedHeartbeatIntervals?: number;
  /** True when the claim's lease is near expiry without progress. */
  leaseNearExpiryWithoutProgress?: boolean;
  /** Requeues/retries in the trailing 14 days. */
  requeues14d?: number;
}

export interface StallRiskInput {
  ref: RefKey;
  cutoffs: ItemHistoryCutoff[];
  /** Meaningful-activity proxy: the item's `updatedAt`. The design
   * is explicit that until event typing exists this is a proxy, and
   * the reason code says so. Omit to leave the signal unknown. */
  updatedAt?: string | null;
  /** Queue heartbeat/requeue facts, when a queue source supplies
   * them for this item. Omit to leave both signals unknown. */
  queue?: QueueSignals;
  /** Evaluation instant; defaults to the latest complete cutoff. */
  asOf?: string;
}

export interface StallSignal {
  code: "blocked_edge_age" | "no_meaningful_activity" | "queue_heartbeat" | "queue_requeues";
  /** Points contributed (0 when the signal did not fire). */
  contribution: number;
  /** False when no source supplied this signal: it contributed
   * nothing and reduced evidence coverage. */
  known: boolean;
  /** When the contributing observation was made, when it fired. */
  observedAt: string | null;
  detail: string;
}

export interface StallRisk {
  ref: RefKey;
  asOf: string;
  modelVersion: string;
  score: number;
  threshold: number;
  atRisk: boolean;
  /** Always false: score alone never escalates (§3). A concrete
   * blocker or a dead claim enters Needs-you through its own path. */
  escalatesToNeedsYou: false;
  /** Fraction of the four signals a source actually supplied. */
  evidenceCoverage: number;
  signals: StallSignal[];
}

const STALL_WEIGHTS = {
  blocked_edge_age: 30,
  no_meaningful_activity: 20,
  queue_heartbeat: 30,
  queue_requeues: 20,
} as const;

/** Versioned, explainable stall-risk score for one item (§3). */
export function deriveStallRisk(input: StallRiskInput): StallRisk {
  // Same rolling-origin rule as deriveEpicEta: only cutoffs observed
  // at or before asOf take part.
  const scopedCutoffs = input.asOf ? input.cutoffs.filter((c) => atOrBefore(c.cutoffAt, input.asOf!)) : input.cutoffs;
  const complete = completeCutoffs(scopedCutoffs);
  const latest = complete[complete.length - 1];
  const asOf = input.asOf ?? latest?.cutoffAt ?? new Date(0).toISOString();
  // The blocked-edge helper measures age through the last cutoff it is
  // given. Bound its history to the effective asOf: when asOf is
  // omitted, scopedCutoffs can contain later, unavailable cutoffs, and
  // observations from them must not age an edge inside a result dated
  // at the latest complete cutoff.
  const boundedCutoffs = scopedCutoffs.filter((c) => atOrBefore(c.cutoffAt, asOf));
  const signals: StallSignal[] = [];

  // Blocked edge age: lower bound from first observation of the
  // current episode of each edge on the item at the latest cutoff.
  const item = latest?.items.find((i) => i.ref === input.ref);
  if (item && item.blockedBy.length > 0) {
    let fired: StallSignal | null = null;
    let resolvedBlockers = 0;
    for (const blocker of item.blockedBy) {
      // A blocker the latest cutoff holds as closed/merged is resolved
      // (graph.ts treats it the same way), even though the raw
      // blockedBy relation persists in the history. A blocker absent
      // from the pool cannot be shown resolved and stays unresolved.
      const blockerItem = latest.items.find((i) => i.ref === blocker);
      if (blockerItem && blockerItem.state !== "open") {
        resolvedBlockers++;
        continue;
      }
      const first = blockerFirstObserved(boundedCutoffs, input.ref, blocker);
      if (first && first.lowerBoundAgeDays >= 7) {
        fired = {
          code: "blocked_edge_age",
          contribution: STALL_WEIGHTS.blocked_edge_age,
          known: true,
          observedAt: first.firstObservedAt,
          detail: `blocked by ${blocker} for at least ${first.lowerBoundAgeDays} days (first-observed lower bound, not the native edge age)`,
        };
        break;
      }
    }
    signals.push(
      fired ?? {
        code: "blocked_edge_age",
        contribution: 0,
        known: true,
        observedAt: null,
        detail:
          resolvedBlockers > 0
            ? "blocked edges present but every in-pool blocker is resolved at the latest complete cutoff; no out-of-pool blocker was observed unresolved for 7+ days"
            : "blocked edges present but none observed unresolved for 7+ days",
      },
    );
  } else if (item) {
    signals.push({
      code: "blocked_edge_age",
      contribution: 0,
      known: true,
      observedAt: null,
      detail: "no blocked-by edges observed at the latest complete cutoff",
    });
  } else {
    signals.push({
      code: "blocked_edge_age",
      contribution: 0,
      known: false,
      observedAt: null,
      detail: "item not present in the latest complete cutoff; blocker state unknown",
    });
  }

  // Meaningful activity: updatedAt is a proxy until event typing
  // exists, and the detail says so (§3).
  if (input.updatedAt) {
    const ageDays = Math.floor((Date.parse(asOf) - Date.parse(input.updatedAt)) / DAY_MS);
    signals.push({
      code: "no_meaningful_activity",
      contribution: ageDays >= 7 ? STALL_WEIGHTS.no_meaningful_activity : 0,
      known: true,
      observedAt: input.updatedAt,
      detail:
        ageDays >= 7
          ? `no update for ${ageDays} days (updatedAt proxy for meaningful activity; synchronization-only updates are not distinguished yet)`
          : `last update ${Math.max(0, ageDays)} day(s) ago (updatedAt proxy)`,
    });
  } else {
    signals.push({
      code: "no_meaningful_activity",
      contribution: 0,
      known: false,
      observedAt: null,
      detail: "no updatedAt supplied; activity signal unknown",
    });
  }

  // Queue heartbeat / lease signals: known only when a queue source
  // supplied them for this item.
  if (
    input.queue &&
    (input.queue.missedHeartbeatIntervals !== undefined || input.queue.leaseNearExpiryWithoutProgress !== undefined)
  ) {
    const missed = input.queue.missedHeartbeatIntervals ?? 0;
    const nearExpiry = input.queue.leaseNearExpiryWithoutProgress === true;
    signals.push({
      code: "queue_heartbeat",
      contribution: missed >= 2 || nearExpiry ? STALL_WEIGHTS.queue_heartbeat : 0,
      known: true,
      observedAt: asOf,
      detail:
        missed >= 2
          ? `claim missed ${missed} expected heartbeat intervals`
          : nearExpiry
            ? "claim lease approaching expiry without progress"
            : "claim heartbeat within expected intervals",
    });
  } else {
    signals.push({
      code: "queue_heartbeat",
      contribution: 0,
      known: false,
      observedAt: null,
      detail: "no queue heartbeat source for this item; signal unknown",
    });
  }

  if (input.queue && input.queue.requeues14d !== undefined) {
    const n = input.queue.requeues14d;
    signals.push({
      code: "queue_requeues",
      contribution: n >= 2 ? STALL_WEIGHTS.queue_requeues : 0,
      known: true,
      observedAt: asOf,
      detail: `${n} requeue(s)/retries in the trailing 14 days`,
    });
  } else {
    signals.push({
      code: "queue_requeues",
      contribution: 0,
      known: false,
      observedAt: null,
      detail: "no queue requeue source for this item; signal unknown",
    });
  }

  const score = Math.min(
    100,
    signals.reduce((a, s) => a + s.contribution, 0),
  );
  return {
    ref: input.ref,
    asOf,
    modelVersion: STALL_MODEL_VERSION,
    score,
    threshold: STALL_AT_RISK_THRESHOLD,
    atRisk: score >= STALL_AT_RISK_THRESHOLD,
    escalatesToNeedsYou: false,
    evidenceCoverage: signals.filter((s) => s.known).length / signals.length,
    signals,
  };
}
