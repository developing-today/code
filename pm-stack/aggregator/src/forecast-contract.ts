// Shared forecast contract for #3984 (design:
// doc/2026-11-01T21-07-00Z_design_pm-stack_forecasting.md §4, parent
// #1256). Roadmap and Projects render THE SAME machine-readable
// forecast objects this module builds, and GET /api/forecasts.json
// serves them verbatim — page wording and API wording cannot drift.
//
// The three forecast types of the design stay separate objects so
// missing data in one plane can never make another look more certain:
//
// - epic_completion: wraps deriveEpicEta (#3982, src/forecast.ts).
// - quota_exhaustion: a per-pool wall interval from a measured burn
//   rate. The per-pool rate-history model is #3983, still open; until
//   a caller can supply a rate measured in the pool's own units, the
//   honest result is the first-class "quota_rate_unavailable" state —
//   never an invented date and never "safe".
// - stall_risk: wraps deriveStallRisk (#3982 §3). The score is a
//   versioned rule score, not a probability; the contract exposes the
//   contributing signals and the evidence coverage, and a stall score
//   alone never escalates to Needs-you.
//
// Honesty rules (§4): p50/p80 are nullable; "no measurable pace",
// "not enough history", and "quota rate unavailable" are first-class
// states; missing values are never rendered as zero; every object
// names its scope, asOf, model version, input window/coverage, and
// structured reasons/limitations.

import {
  deriveEpicEta,
  deriveStallRisk,
  type BorrowedBaseline,
  type EpicEta,
  type EtaStatus,
  type ForecastConfidence,
  type StallRisk,
} from "./forecast.ts";
import type { ItemHistoryCutoff } from "./item-history.ts";
import type { RefKey } from "./model.ts";

export const FORECAST_CONTRACT_VERSION = "forecast-contract/v1";
export const QUOTA_FORECAST_MODEL_VERSION = "quota-wall/v1";

/** A rate observation older than this is stale source, not a rate.
 * Mirrors QUOTA_CURRENT_FRESH_MS (src/quota-current.ts). */
export const QUOTA_RATE_FRESH_MS = 20 * 60_000;

export interface ForecastReason {
  code: string;
  text: string;
}

interface ForecastContractBase {
  contractVersion: typeof FORECAST_CONTRACT_VERSION;
  subjectRef: RefKey;
  /** The actual pool/subject scope the numbers describe, stated in
   * words — a Project #4 pool scope is a curated subset, never
   * silently a repo-wide backlog (§4 / design "Current inputs"). */
  scope: string;
  asOf: string;
  modelVersion: string;
  /** Evidence grade, not a probability (design §1). */
  confidence: ForecastConfidence;
  reasons: ForecastReason[];
  limitations: ForecastReason[];
}

export interface EpicCompletionForecast extends ForecastContractBase {
  type: "epic_completion";
  status: EtaStatus;
  inputWindow: EpicEta["inputWindow"];
  inputCoverage: EpicEta["inputCoverage"];
  remainingUnits: number;
  completedUnits: number;
  /** ISO dates, or null when no defensible date exists. */
  p50: string | null;
  p80: string | null;
  baselineSource: string;
  remainingEffort: EpicEta["remainingEffort"];
}

export type QuotaForecastStatus = "forecast" | "quota_rate_unavailable" | "no_recent_burn";

export interface QuotaExhaustionForecast extends ForecastContractBase {
  type: "quota_exhaustion";
  status: QuotaForecastStatus;
  poolId: string;
  provider: string | null;
  unit: string;
  remainingAmount: number | null;
  limitAmount: number | null;
  nextReset: string | null;
  inputWindow: { from: string | null; to: string };
  inputCoverage: { rateKnown: boolean; missing: string[] };
  /** Exhaustion interval from the observed rate; null unless a fresh
   * measured rate, a remaining amount, and a reset are all known. */
  exhaustion: {
    estimatedAt: string;
    earliest: string;
    latest: string;
    beforeReset: boolean;
  } | null;
}

export interface StallRiskForecast extends ForecastContractBase {
  type: "stall_risk";
  /** Versioned rule score 0–100. NOT a probability/percentage. */
  score: number;
  threshold: number;
  atRisk: boolean;
  /** Always false: score alone never escalates (§3). */
  escalatesToNeedsYou: false;
  /** Fraction of the four stall signals a source actually supplied. */
  evidenceCoverage: number;
  signals: StallRisk["signals"];
}

export type ForecastContract = EpicCompletionForecast | QuotaExhaustionForecast | StallRiskForecast;

const limitation = (text: string): ForecastReason => ({ code: "model_limitation", text });

export interface EpicForecastOptions {
  scope?: string;
  /** GitHub-vs-Project status disagreement for the subject, in words,
   * when the caller knows one — surfaced on the contract, never
   * resolved silently (§4). */
  statusDisagreement?: string | null;
}

/** Map a derived epic ETA (#3982) onto the shared contract (§4). */
export function epicForecastContract(eta: EpicEta, opts: EpicForecastOptions = {}): EpicCompletionForecast {
  const reasons: ForecastReason[] = [];
  if (eta.status === "borrowed_baseline") {
    reasons.push({
      code: "borrowed_baseline",
      text: `local evidence is below the numeric-forecast floor; the rate is borrowed from ${eta.baselineSource}`,
    });
  } else if (eta.status === "not_enough_history") {
    reasons.push({
      code: "not_enough_history",
      text: "not enough complete observed history for a defensible date; no date is published",
    });
  } else if (eta.status === "no_measurable_pace") {
    reasons.push({
      code: "no_measurable_pace",
      text: "the observed completion pace in scope is zero; no date is published",
    });
  }
  if (opts.statusDisagreement) {
    reasons.push({ code: "status_disagreement", text: opts.statusDisagreement });
  }
  return {
    contractVersion: FORECAST_CONTRACT_VERSION,
    type: "epic_completion",
    subjectRef: eta.subjectRef,
    scope: opts.scope ?? `direct native children of ${eta.subjectRef} in the item-history pool`,
    asOf: eta.asOf,
    modelVersion: eta.modelVersion,
    confidence: eta.confidence,
    reasons,
    limitations: eta.limitations.map(limitation),
    status: eta.status,
    inputWindow: eta.inputWindow,
    inputCoverage: eta.inputCoverage,
    remainingUnits: eta.remainingUnits,
    completedUnits: eta.completedUnits,
    p50: eta.p50,
    p80: eta.p80,
    baselineSource: eta.baselineSource,
    remainingEffort: eta.remainingEffort,
  };
}

export interface QuotaForecastInput {
  poolId: string;
  provider?: string | null;
  subjectRef?: RefKey;
  scope?: string;
  asOf: string;
  unit: string;
  remainingAmount?: number | null;
  limitAmount?: number | null;
  nextReset?: string | null;
  /** Burn measured in the pool's own units. Omit (or pass null) when
   * no source measures the rate in these units — the result is then
   * "quota_rate_unavailable", by design (§2). */
  rate?: {
    amountPerHour: number;
    observedAt: string;
    /** Observed low/high rate band, when a rate history supplies one
     * (#3933/#3983). Without it the interval collapses to the point
     * estimate and says so in limitations. */
    intervalPerHour?: { low: number; high: number };
  } | null;
  /** Max acceptable rate age; defaults to QUOTA_RATE_FRESH_MS. */
  maxRateAgeMs?: number;
}

/** Build the quota-wall contract for one pool (§2 + §4). */
export function quotaForecastContract(input: QuotaForecastInput): QuotaExhaustionForecast {
  const reasons: ForecastReason[] = [];
  const limitations: ForecastReason[] = [];
  const missing: string[] = [];
  const remaining = input.remainingAmount ?? null;
  const limit = input.limitAmount ?? null;
  const nextReset = input.nextReset ?? null;
  if (remaining === null) missing.push("remaining amount");
  if (limit === null) missing.push("limit");
  if (nextReset === null) missing.push("next reset");
  const rate = input.rate ?? null;
  if (rate === null) missing.push("measured burn rate");

  let status: QuotaForecastStatus = "quota_rate_unavailable";
  let exhaustion: QuotaExhaustionForecast["exhaustion"] = null;
  if (rate !== null) {
    const ageMs = Date.parse(input.asOf) - Date.parse(rate.observedAt);
    if (Number.isNaN(ageMs) || ageMs < 0 || ageMs > (input.maxRateAgeMs ?? QUOTA_RATE_FRESH_MS)) {
      reasons.push({
        code: "stale_source",
        text: `burn rate observed at ${rate.observedAt} is stale at ${input.asOf}; a stale rate publishes no date`,
      });
      missing.push("fresh burn rate");
    } else if (!(rate.amountPerHour > 0)) {
      status = "no_recent_burn";
      reasons.push({
        code: "no_recent_burn",
        text: "measured burn rate is zero; absence of burn is not evidence the pool is safe",
      });
    } else if (remaining !== null && nextReset !== null) {
      const at = (hours: number) => new Date(Date.parse(input.asOf) + hours * 3_600_000).toISOString();
      const estimatedAt = at(remaining / rate.amountPerHour);
      let earliest = estimatedAt;
      let latest = estimatedAt;
      const band = rate.intervalPerHour;
      if (band && band.low > 0 && band.high >= band.low) {
        earliest = at(remaining / band.high);
        latest = at(remaining / band.low);
      } else {
        limitations.push({
          code: "single_linear_rate",
          text: "single linear rate: the interval collapses to the point estimate until a measured rate history (#3933/#3983) supplies a band; no statistical confidence is implied",
        });
      }
      exhaustion = { estimatedAt, earliest, latest, beforeReset: estimatedAt <= nextReset };
      status = "forecast";
      reasons.push({
        code: exhaustion.beforeReset ? "wall_before_reset" : "wall_after_reset",
        text: exhaustion.beforeReset
          ? "the measured rate exhausts the remaining amount before the next reset"
          : "the measured rate does not exhaust the remaining amount before the next reset",
      });
    }
  }
  if (status === "quota_rate_unavailable") {
    for (const m of [...new Set(missing)]) {
      reasons.push({ code: `missing_${m.replace(/[^a-z]+/g, "_")}`, text: `${m} unavailable for pool ${input.poolId}` });
    }
    if (!reasons.some((r) => r.code === "stale_source")) {
      reasons.push({
        code: "quota_rate_unavailable",
        text: "quota rate unavailable: no exhaustion date is published and the pool is not shown as safe",
      });
    }
  }
  return {
    contractVersion: FORECAST_CONTRACT_VERSION,
    type: "quota_exhaustion",
    subjectRef: input.subjectRef ?? `quota:${input.poolId}`,
    scope: input.scope ?? `quota pool ${input.poolId}`,
    asOf: input.asOf,
    modelVersion: QUOTA_FORECAST_MODEL_VERSION,
    confidence: status === "forecast" ? "low" : "unavailable",
    reasons,
    limitations,
    status,
    poolId: input.poolId,
    provider: input.provider ?? null,
    unit: input.unit,
    remainingAmount: remaining,
    limitAmount: limit,
    nextReset,
    inputWindow: { from: rate?.observedAt ?? null, to: input.asOf },
    inputCoverage: { rateKnown: rate !== null && status !== "quota_rate_unavailable", missing: [...new Set(missing)] },
    exhaustion,
  };
}

export interface StallForecastOptions {
  scope?: string;
}

/** Map a derived stall-risk score (#3982 §3) onto the shared contract. */
export function stallForecastContract(risk: StallRisk, opts: StallForecastOptions = {}): StallRiskForecast {
  const reasons: ForecastReason[] = risk.signals
    .filter((s) => s.known && s.contribution > 0)
    .map((s) => ({ code: s.code, text: s.detail }));
  const limitations: ForecastReason[] = risk.signals
    .filter((s) => !s.known)
    .map((s) => ({ code: "signal_unknown", text: `${s.code}: no source supplied this signal; it contributed nothing` }));
  return {
    contractVersion: FORECAST_CONTRACT_VERSION,
    type: "stall_risk",
    subjectRef: risk.ref,
    scope: opts.scope ?? `item ${risk.ref}`,
    asOf: risk.asOf,
    modelVersion: risk.modelVersion,
    confidence: risk.evidenceCoverage >= 1 ? "medium" : risk.evidenceCoverage > 0 ? "low" : "unavailable",
    reasons,
    limitations,
    score: risk.score,
    threshold: risk.threshold,
    atRisk: risk.atRisk,
    escalatesToNeedsYou: false,
    evidenceCoverage: risk.evidenceCoverage,
    signals: risk.signals,
  };
}

export interface SubjectForecastOptions extends EpicForecastOptions {
  /** Meaningful-activity proxy for the stall plane: the item's
   * updatedAt, when the caller has it. */
  updatedAt?: string | null;
  borrowedBaseline?: BorrowedBaseline;
}

/** The epic-completion + stall-risk contracts for one subject, from
 * the stored item history. Both planes share the same cutoffs; each
 * keeps its own honesty state. */
export function subjectForecastContracts(
  cutoffs: ItemHistoryCutoff[],
  subjectRef: RefKey,
  opts: SubjectForecastOptions = {},
): ForecastContract[] {
  const eta = deriveEpicEta({ subjectRef, cutoffs, borrowedBaseline: opts.borrowedBaseline });
  const stall = deriveStallRisk({ ref: subjectRef, cutoffs, updatedAt: opts.updatedAt ?? null });
  return [epicForecastContract(eta, opts), stallForecastContract(stall, opts)];
}

// --- Shared one-line rendering (§4 presentation) --------------------------

function fmtDay(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime())
    ? iso
    : d.toLocaleDateString("en-US", { month: "short", day: "numeric", timeZone: "UTC" });
}

function fmtRange(from: string, to: string): string {
  return from === to ? fmtDay(from) : `${fmtDay(from)} – ${fmtDay(to)}`;
}

/** One line, always carrying its evidence beside the estimate: a
 * range (or a named no-date state), the confidence grade, and the
 * input summary. Never a bare date, never a missing value as zero. */
export function formatForecastSummary(contract: ForecastContract): string {
  if (contract.type === "epic_completion") {
    const total = contract.completedUnits + contract.remainingUnits;
    const evidence =
      `${contract.completedUnits} of ${total} completed children, ${contract.inputWindow.completeWeeks} complete observed weeks; ` +
      `baseline: ${contract.baselineSource}`;
    if (contract.p50 && contract.p80) {
      return `${fmtRange(contract.p50, contract.p80)} · ${contract.confidence} confidence · ${evidence}`;
    }
    if (contract.status === "no_measurable_pace") return `No measurable pace · ${evidence}`;
    if (contract.status === "not_enough_history") return `Not enough history · ${evidence}`;
    return `No forecast date · ${contract.confidence} confidence · ${evidence}`;
  }
  if (contract.type === "quota_exhaustion") {
    if (contract.status === "forecast" && contract.exhaustion) {
      const reset = contract.nextReset ? `, resets ${fmtDay(contract.nextReset)}` : "";
      return (
        `Quota wall ${fmtRange(contract.exhaustion.earliest, contract.exhaustion.latest)} · low confidence · ` +
        `${contract.remainingAmount} of ${contract.limitAmount} ${contract.unit} remaining${reset}; ` +
        `${contract.exhaustion.beforeReset ? "exhausts before reset" : "does not exhaust before reset"}`
      );
    }
    if (contract.status === "no_recent_burn") {
      return `No recent burn measured · pool ${contract.poolId} · not evidence the pool is safe`;
    }
    return `Quota rate unavailable · pool ${contract.poolId} · no exhaustion date published`;
  }
  const signals = contract.reasons.length
    ? contract.reasons.map((r) => r.text).join("; ")
    : "no contributing signals";
  const known = contract.signals.filter((sig) => sig.known).length;
  return (
    `Stall risk ${contract.score} (threshold ${contract.threshold}) · ${contract.atRisk ? "at risk" : "below threshold"} · ` +
    `${signals} · evidence from ${known} of ${contract.signals.length} signals`
  );
}
