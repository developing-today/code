// Full-quota utilization: the projection model behind the standing plan
// and the 4–6h check (#2173).
//
// Drew's standing rule (#2173, 2026-10-29): if a plan is not on track to
// be used in full by its reset, queue more useful work — leftover quota
// is waste. That needs three things this module supplies as pure
// functions over the recorded quota series:
//
//   1. A measured burn rate per pool — never an assumed one. The series
//      is the ledger (Turso agent_quota_records, source "opencode-quota",
//      written by scripts/gather-quota.ts every 15 minutes); the rate is
//      the Theil–Sen slope of the *current* window segment, so one noisy
//      tick cannot move it and a mid-series reset cannot poison it.
//   2. A projection per pool's LARGEST window: current %, reset instant,
//      the rate that lands it at ~100% exactly at reset, the landing at
//      the measured rate, and the landing in the period after next.
//   3. A cascade model: where the pool sits in the routing fallback map,
//      and how much work spills to its successor when it walls early.
//
// Everything here is pure: the CLI shell (scripts/quota-full-utilization-
// plan.ts) owns reading the ledger and writing the plan/check records.
// Tests pin the semantics (test/quota-plan.test.ts), including an
// independent recomputation of the projection as the oracle.

/** Why a pool is where it is. One verdict per pool, per check. */
export type Verdict =
  | "on_track"
  | "under_pace"
  | "over_pace"
  | "exhausted"
  | "no_reset"
  | "no_data"
  | "stale";

/** One window reading inside one pool sample. */
export interface RawWindow {
  name: string;
  percent: number | null;
  resetsAt: string | null;
}

/** One recorded pool reading (one ledger row, decoded). */
export interface RawSample {
  /** Stable pool id, e.g. "opencode-go.drewrypope". */
  pool: string;
  /** Provider label as the ledger carries it, e.g. "OpenCode Go". */
  provider: string;
  /** Plan label when the ledger carries one, e.g. "Muse Code High Usage". */
  plan: string | null;
  /** Reading instant (ISO). */
  ts: string;
  windows: RawWindow[];
}

export interface RateEstimate {
  /** Percentage points of the window consumed per hour over the current period. */
  perHour: number;
  /** The same measure over the trailing 24h — acceleration/deceleration signal. */
  recentPerHour: number;
  /** Hours between the first and last sample used. */
  spanHours: number;
  /** How many samples the estimate used. */
  samples: number;
  /**
   * "ok" — at least 3 samples over >= 2h of the current segment.
   * "low" — fewer samples or a shorter segment (still reported, flagged).
   * "none" — no usable pair of samples.
   */
  confidence: "ok" | "low" | "none";
}

export interface CascadePosition {
  /** Capability group the pool serves (routing fallback family). */
  group: string;
  /** 1-based position within the group. */
  position: number;
  /** Group size. */
  of: number;
  /** Pool work falls through to when this one walls; null = last resort. */
  next: string | null;
}

export interface Audible {
  /** The instant the audible is due (ISO). */
  at: string;
  /** Stable action id: confirm | queue-catch-up | flag-drew | repair-reading | plan-next-period. */
  action: "confirm" | "queue-catch-up" | "flag-drew" | "repair-reading" | "plan-next-period";
  /** One-line, dated instruction for the operator/lane. */
  detail: string;
  /** Last instant an action can still change the landing (ISO), when one exists. */
  deadline?: string;
}

export interface PoolProjection {
  pool: string;
  provider: string;
  plan: string | null;
  window: string;
  windowSeconds: number | null;
  /** Latest measured percentage for the chosen window. */
  currentPercent: number;
  /** Instant of the latest sample used. */
  measuredAt: string;
  /** Reset instant for the chosen window, when the reading carries one. */
  resetsAt: string | null;
  /** Hours until reset at `asOf`; negative when the reset instant has passed. */
  hoursToReset: number | null;
  rate: RateEstimate;
  /** Rate needed to land exactly on 100% at the reset, in points/hour. */
  requiredPerHour: number | null;
  /** Landing at reset if the measured rate holds. */
  projectedLanding: number | null;
  /** Landing in the reset-after-next period at the measured rate. */
  projectedLandingNextPeriod: number | null;
  /** When the pool walls at the measured rate, if it does before its reset. */
  exhaustionAt: string | null;
  verdict: Verdict;
  reason: string;
  cascade: CascadePosition;
  /** Work spilled to the cascade successor, in percentage points of this pool's window. */
  spillPctPoints: number;
  audible: Audible | null;
  notes: string[];
}

export interface FullUtilizationPlan {
  version: 1;
  /** When the plan was computed (ISO). */
  generatedAt: string;
  /** The instant projections were made as of (ISO; replay may differ from generatedAt). */
  asOf: string;
  /** Shipped method parameters, so a plan is reproducible from its own record. */
  method: PlanMethod;
  pools: PoolProjection[];
  /** Pools reported for visibility but not judged (infrastructure meters, no percent). */
  resourcePools: { pool: string; provider: string; windows: string[]; note: string }[];
  dataQuality: string[];
  summary: {
    underPace: number;
    overPace: number;
    exhausted: number;
    onTrack: number;
    stale: number;
    noReset: number;
    noData: number;
  };
}

export interface PlanMethod {
  /**
   * Cap on the span of the current window segment used for the burn
   * rate. The rate is measured over the whole current period (so a
   * monthly meter is not judged on one busy afternoon), capped here so
   * month-old history cannot dominate.
   */
  rateWindowHours: number;
  /** A landing below (100 - underPaceTolerance) points is under pace. */
  underPaceTolerance: number;
  /** A landing above (100 + overPaceTolerance) points is over pace. */
  overPaceTolerance: number;
  /** A reading older than this many hours is stale, whatever it says. */
  staleHours: number;
  /** The check cadence the plan is written for. */
  checkIntervalHours: number;
}

export const DEFAULT_METHOD: PlanMethod = {
  rateWindowHours: 168,
  underPaceTolerance: 10,
  overPaceTolerance: 5,
  staleHours: 6,
  checkIntervalHours: 6,
};

/**
 * The routing fallback map: which capacity a pool's work falls through
 * to when the pool walls. Derived from the provider-quota-registry's
 * burn order and the pools the quota gatherer actually reports; it is a
 * plan input, not a provider configuration — changing weights or routes
 * remains Drew's call, and this map only models the consequence.
 *
 * Pools absent from every group are reported as their own single-entry
 * group (position 1 of 1, no successor).
 */
export const CASCADE_GROUPS: Record<string, string[]> = {
  // DeepSeek-class work: the OpenCode Go workspaces serve the same
  // DeepSeek/Qwen/GLM pool family, then the DeepSeek API credits.
  "deepseek-class": ["opencode-go.drewrypope", "opencode-go.developing-today-0", "deepseek"],
  // Gemini-class work through Antigravity. No sibling provider serves
  // this family, so the last entry has no successor.
  "gemini-class": ["_antigravity.gemini"],
  // Claude-class work: the Claude Max plan first, then Antigravity's
  // Claude/GPT allowance.
  "claude-class": ["anthropic:claude-pro", "_antigravity.claude-gpt"],
  // GPT-class work: the Codex main account, then the alternate account,
  // then Antigravity's Claude/GPT allowance.
  "gpt-class": ["openai.main", "openai.alternate", "_antigravity.claude-gpt"],
  // Meta's plan has no sibling; its overflow historically spilled to PAYG.
  "meta-class": ["meta"],
  // Standing last-resort model pools.
  "misc-class": ["cline-pass", "_commandcode", "mistral", "opendesign"],
};

/**
 * Combined/billing rows the quota plane already excludes from its pool
 * list (quota-current.ts NON_POOL_SERVICE_IDS): they overlay the split
 * rows, so planning them would double-count the same allowance.
 */
export const NON_POOL_SERVICE_IDS: ReadonlySet<string> = new Set([
  "turso",
  "antigravity",
  "_antigravity",
  "opencode-go",
]);

/** Frozen history ids: no longer written by the gatherer, never planned. */
export const LEGACY_SERVICE_IDS: ReadonlySet<string> = new Set([
  "antigravity-direct",
  "antigravity-direct.gemini",
  "antigravity-direct.claude-gpt",
  "antigravity-direct.antigravity",
  "commandcode",
  "pm-pool",
]);

/** Pools that are meters of infrastructure, not model quota. */
export const RESOURCE_POOLS: Record<string, string> = {
  turso: "Turso database/storage/sync meters — infra capacity, not a model pool.",
};

/** Window-name to duration in seconds. Largest window wins the plan row. */
export function windowSeconds(name: string): number | null {
  const n = name.toLowerCase();
  if (n === "5h") return 5 * 3600;
  if (n === "7d" || n === "weekly") return 7 * 24 * 3600;
  if (n === "monthly") return 30 * 24 * 3600;
  if (n === "rolling") return 7 * 24 * 3600;
  // Per-model windows in the Antigravity summary are "<model>:weekly" /
  // "<model>:5h".
  const tail = n.includes(":") ? n.slice(n.lastIndexOf(":") + 1) : null;
  if (tail && tail !== n) return windowSeconds(tail);
  return null;
}

/** Pick the pool's largest measured window by duration; null when none is known. */
export function pickLargestWindow(windows: RawWindow[]): string | null {
  let best: { name: string; seconds: number } | null = null;
  for (const w of windows) {
    if (w.percent === null) continue;
    const seconds = windowSeconds(w.name);
    if (seconds === null) continue;
    if (!best || seconds > best.seconds || (seconds === best.seconds && w.name < best.name)) {
      best = { name: w.name, seconds };
    }
  }
  return best?.name ?? null;
}

/** Theil–Sen slope (median of pairwise slopes) in points per hour. */
export function theilSenSlope(points: { ts: number; percent: number }[]): number | null {
  if (points.length < 2) return null;
  const slopes: number[] = [];
  for (let i = 0; i < points.length; i += 1) {
    for (let j = i + 1; j < points.length; j += 1) {
      const dt = (points[j].ts - points[i].ts) / 3_600_000;
      if (dt <= 0) continue;
      slopes.push((points[j].percent - points[i].percent) / dt);
    }
  }
  if (!slopes.length) return null;
  slopes.sort((a, b) => a - b);
  const mid = Math.floor(slopes.length / 2);
  return slopes.length % 2 ? slopes[mid] : (slopes[mid - 1] + slopes[mid]) / 2;
}

function median(values: number[]): number {
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2;
}

/**
 * Burn slope between the trimmed endpoints of a segment, in points/hour.
 *
 * Quota meters are step functions: a provider refreshes its percentage
 * every few minutes to an hour, so most adjacent samples are equal and a
 * median-of-pairwise-slopes estimator is dragged toward zero by the flat
 * steps (it reports ~0 for a pool that has just burned 8 points). The
 * net-burn slope is the honest reading of "what this pool is consuming
 * per hour": the change between the segment's two ends, over the time
 * between them. Each end is the mean of up to three samples with the
 * single most extreme value dropped, so one bad tick at either end
 * cannot move the whole projection.
 */
function robustEndpoint(points: { ts: number; percent: number }[]): { ts: number; percent: number } {
  if (points.length <= 2) {
    return {
      ts: points.reduce((a, p) => a + p.ts, 0) / points.length,
      percent: points.reduce((a, p) => a + p.percent, 0) / points.length,
    };
  }
  const mid = median(points.map((p) => p.percent));
  let worst = 0;
  for (let i = 1; i < points.length; i += 1) {
    if (Math.abs(points[i].percent - mid) > Math.abs(points[worst].percent - mid)) worst = i;
  }
  const kept = points.filter((_, i) => i !== worst);
  return {
    ts: kept.reduce((a, p) => a + p.ts, 0) / kept.length,
    percent: kept.reduce((a, p) => a + p.percent, 0) / kept.length,
  };
}

export function netBurnSlope(points: { ts: number; percent: number }[]): number | null {
  if (points.length < 2) return null;
  const k = Math.min(3, Math.floor(points.length / 2));
  const head = robustEndpoint(points.slice(0, k));
  const tail = robustEndpoint(points.slice(-k));
  const hours = (tail.ts - head.ts) / 3_600_000;
  if (hours <= 0) return null;
  return (tail.percent - head.percent) / hours;
}

/**
 * Measure the burn rate over the current window segment.
 *
 * A reset inside the candidate range shows up as a material drop in the
 * window percentage; only samples after the LAST such drop are used, so
 * a pool that reset mid-morning is measured from its new period (and is
 * never reported as burning backwards). Estimates never go negative: a
 * small dip (a corrected reading, a rounding step) is a zero rate plus a
 * note, not negative burn.
 */
export function estimateRate(
  all: { ts: number; percent: number }[],
  opts: { asOf: number; rateWindowHours: number; resetDropPts?: number; minSpanHours?: number },
): { rate: RateEstimate; pointsUsed: number; resetDetected: boolean } {
  const resetDropPts = opts.resetDropPts ?? 5;
  const minSpanHours = opts.minSpanHours ?? 2;
  const sorted = [...all].filter((p) => p.ts <= opts.asOf).sort((a, b) => a.ts - b.ts);

  // Keep only the segment after the last material drop.
  let start = 0;
  for (let i = 1; i < sorted.length; i += 1) {
    if (sorted[i - 1].percent - sorted[i].percent > resetDropPts) start = i;
  }
  const resetDetected = start > 0;
  let segment = sorted.slice(start);

  // Trail only the rate window, but never discard the whole segment.
  const cutoff = opts.asOf - opts.rateWindowHours * 3_600_000;
  const trailed = segment.filter((p) => p.ts >= cutoff);
  if (trailed.length >= 2) segment = trailed;

  if (segment.length < 2) {
    return {
      rate: { perHour: 0, recentPerHour: 0, spanHours: 0, samples: segment.length, confidence: "none" },
      pointsUsed: segment.length,
      resetDetected,
    };
  }

  const spanHours = (segment[segment.length - 1].ts - segment[0].ts) / 3_600_000;
  const slope = netBurnSlope(segment) ?? 0;
  const perHour = Math.max(0, slope);

  // The trailing day, measured the same way, is the acceleration signal.
  const recentCutoff = opts.asOf - 24 * 3_600_000;
  const recent = segment.filter((p) => p.ts >= recentCutoff);
  const recentPerHour = recent.length >= 2 ? Math.max(0, netBurnSlope(recent) ?? 0) : perHour;

  const confidence: RateEstimate["confidence"] = segment.length >= 3 && spanHours >= minSpanHours ? "ok" : "low";
  return {
    rate: { perHour, recentPerHour, spanHours, samples: segment.length, confidence },
    pointsUsed: segment.length,
    resetDetected,
  };
}

/** Where a pool sits in the routing fallback map. */
export function cascadeFor(pool: string, groups: Record<string, string[]> = CASCADE_GROUPS): CascadePosition {
  for (const [group, members] of Object.entries(groups)) {
    const idx = members.indexOf(pool);
    if (idx >= 0) {
      return { group, position: idx + 1, of: members.length, next: members[idx + 1] ?? null };
    }
  }
  return { group: "ungrouped", position: 1, of: 1, next: null };
}

export interface ProjectOptions {
  method?: Partial<PlanMethod>;
  /** Override the routing fallback map (tests, or a maintenance window). */
  cascadeGroups?: Record<string, string[]>;
  /** Extra pools to skip entirely (besides RESOURCE_POOLS). */
  skipPools?: string[];
  /** Plan the combined/billing rows too (they overlay the split pools). */
  includeNonPools?: boolean;
}

function nextCheckInstant(asOf: number, hours: number): string {
  return new Date(asOf + hours * 3_600_000).toISOString();
}

function round(n: number, places = 2): number {
  const f = 10 ** places;
  return Math.round(n * f) / f;
}

/**
 * Project one pool's largest window as of `asOf`.
 *
 * `samples` is the pool's whole recorded series (any order). The latest
 * sample at or before `asOf` supplies the current reading and reset
 * instant; the current-period segment supplies the rate.
 */
export function projectPool(
  pool: string,
  samples: RawSample[],
  asOf: number,
  opts: ProjectOptions = {},
): PoolProjection | null {
  const method: PlanMethod = { ...DEFAULT_METHOD, ...(opts.method ?? {}) };
  const groups = opts.cascadeGroups ?? CASCADE_GROUPS;
  const cascade = cascadeFor(pool, groups);

  const usable = samples
    .filter((s) => Date.parse(s.ts) <= asOf)
    .sort((a, b) => Date.parse(a.ts) - Date.parse(b.ts));
  if (!usable.length) return null;

  const latest = usable[usable.length - 1];
  const provider = latest.provider;
  const windowName = pickLargestWindow(latest.windows);
  if (!windowName) return null;
  const latestWindow = latest.windows.find((w) => w.name === windowName);
  if (!latestWindow || latestWindow.percent === null) return null;
  const seconds = windowSeconds(windowName);
  const currentPercent = latestWindow.percent;
  const measuredAt = Date.parse(latest.ts);
  const resetsAt = latestWindow.resetsAt;
  const resetsMs = resetsAt ? Date.parse(resetsAt) : null;
  const hoursToReset = resetsMs === null || Number.isNaN(resetsMs) ? null : (resetsMs - asOf) / 3_600_000;

  const points = usable
    .map((s) => {
      const w = s.windows.find((x) => x.name === windowName);
      if (!w || w.percent === null) return null;
      return { ts: Date.parse(s.ts), percent: w.percent };
    })
    .filter((p): p is { ts: number; percent: number } => p !== null);

  // Only the current period is a valid rate basis when the reset instant
  // is known: samples before periodStart belong to an earlier period.
  const periodStart = resetsMs !== null && seconds !== null ? resetsMs - seconds * 1000 : null;
  const inPeriod = periodStart === null ? points : points.filter((p) => p.ts >= periodStart);
  const { rate, resetDetected } = estimateRate(inPeriod, {
    asOf,
    rateWindowHours: method.rateWindowHours,
  });

  const notes: string[] = [];
  if (rate.confidence === "low") notes.push("burn rate from a short sample span — treat as provisional");
  if (rate.confidence === "none") notes.push("no usable burn samples in the current period");
  if (
    rate.perHour > 0 &&
    rate.recentPerHour > 0 &&
    Math.max(rate.recentPerHour, rate.perHour) / Math.min(rate.recentPerHour, rate.perHour) > 1.5
  ) {
    notes.push(
      `burn ${rate.recentPerHour > rate.perHour ? "accelerated" : "eased"}: ` +
        `${round(rate.recentPerHour * 24, 2)} pts/day over the trailing 24h vs ${round(rate.perHour * 24, 2)} pts/day over the period`,
    );
  }
  if (resetDetected) notes.push("a reset inside the recorded series was detected; rate uses the current period only");
  if (seconds === null) notes.push("window duration unknown — largest-window selection skipped it");

  const ageHours = (asOf - measuredAt) / 3_600_000;
  const requiredPerHour =
    hoursToReset !== null && hoursToReset > 0 ? Math.max(0, (100 - currentPercent) / hoursToReset) : null;
  const projectedLanding =
    hoursToReset !== null && hoursToReset > 0 ? round(currentPercent + rate.perHour * hoursToReset) : null;
  const projectedLandingNextPeriod =
    seconds !== null ? round(rate.perHour * (seconds / 3600)) : null;

  let exhaustionAt: string | null = null;
  if (hoursToReset !== null && hoursToReset > 0 && rate.perHour > 0) {
    const hoursToWall = Math.max(0, (100 - currentPercent) / rate.perHour);
    if (hoursToWall < hoursToReset) exhaustionAt = new Date(asOf + hoursToWall * 3_600_000).toISOString();
  } else if (hoursToReset !== null && hoursToReset > 0 && currentPercent >= 100) {
    exhaustionAt = new Date(asOf).toISOString();
  }

  const periodElapsedHours =
    seconds !== null && hoursToReset !== null ? seconds / 3600 - hoursToReset : null;

  let verdict: Verdict;
  let reason: string;
  if (ageHours > method.staleHours) {
    verdict = "stale";
    reason = `latest reading is ${round(ageHours, 1)}h old (stale after ${method.staleHours}h)`;
  } else if (hoursToReset === null) {
    verdict = "no_reset";
    reason = resetsAt ? "reset instant unreadable" : "reading carries no reset instant, so no landing is projectable";
  } else if (hoursToReset <= 0.5) {
    verdict = "on_track";
    reason = "reset is at hand; no action window left in this period";
  } else if (periodElapsedHours !== null && periodElapsedHours < 1 && currentPercent < 5) {
    verdict = "on_track";
    reason = "period just started — too early to judge this pool";
  } else if (currentPercent >= 99.5) {
    verdict = "exhausted";
    reason = `fully used for this period (${round(currentPercent, 1)}%) with ${round(hoursToReset, 1)}h to reset`;
  } else if (rate.confidence === "none" || rate.perHour === 0) {
    verdict = "under_pace";
    reason = `no burn measured in the current period (${round(currentPercent, 1)}% used, ${round(hoursToReset, 1)}h left)`;
  } else if (projectedLanding !== null && projectedLanding >= 100 + method.overPaceTolerance) {
    verdict = "over_pace";
    reason = `projects to ${round(projectedLanding, 0)}% at reset — walls ${round((hoursToReset ?? 0) - (exhaustionAt ? (Date.parse(exhaustionAt) - asOf) / 3_600_000 : 0), 1)}h early`;
  } else if (projectedLanding !== null && projectedLanding <= 100 - method.underPaceTolerance) {
    verdict = "under_pace";
    reason = `projects to ${round(projectedLanding, 0)}% at reset — ${round(100 - projectedLanding, 0)} points would be wasted`;
  } else {
    verdict = "on_track";
    reason = `projects to ${round(projectedLanding ?? currentPercent, 0)}% at reset`;
  }

  // Spill is the work that will still arrive after this pool walls inside
  // its current period. An exhausted pool has already cascaded, so only a
  // live over-pace projection carries a spill.
  const spillPctPoints =
    verdict === "over_pace" && projectedLanding !== null ? round(Math.max(0, projectedLanding - 100)) : 0;

  const checkAt = nextCheckInstant(asOf, method.checkIntervalHours);
  let audible: Audible | null;
  switch (verdict) {
    case "under_pace": {
      const shortfall = projectedLanding === null ? null : round(100 - projectedLanding, 0);
      audible = {
        at: checkAt,
        action: "queue-catch-up",
        detail:
          `queue ${shortfall === null ? "catch-up" : `${shortfall}-point catch-up`} work for ${provider} ` +
          `(${windowName}) — lift measured burn from ${round(rate.perHour * 24, 2)} to ` +
          `${round((requiredPerHour ?? 0) * 24, 2)} points/day by the next check; ` +
          `name the destination queue and the ${provider} model class in the work item`,
        ...(hoursToReset !== null && hoursToReset > method.checkIntervalHours
          ? { deadline: new Date(asOf + (hoursToReset - method.checkIntervalHours) * 3_600_000).toISOString() }
          : {}),
      };
      break;
    }
    case "over_pace":
      audible = {
        at: new Date(asOf).toISOString(),
        action: "flag-drew",
        detail:
          `FLAG DREW: ${pool} (${windowName}) projects to exhaust early` +
          (exhaustionAt ? ` at ${exhaustionAt}` : "") +
          `; ask slow-down or keep-progressing (default bias: keep useful work, then cascade to ${cascade.next ?? "no successor"})`,
      };
      break;
    case "exhausted":
      audible = {
        at: checkAt,
        action: "confirm",
        detail:
          `${pool} (${windowName}) is fully used for this period; until the ${resetsAt ?? "next"} reset, ` +
          `${cascade.next ? `route its work to ${cascade.next}` : "it has no cascade successor — queue nothing for it"}`,
      };
      break;
    case "stale":
    case "no_data":
    case "no_reset":
      audible = {
        at: checkAt,
        action: "repair-reading",
        detail: `repair the ${pool} quota reading (${reason}) before the next check can judge it`,
      };
      break;
    default:
      audible = {
        at: checkAt,
        action: "confirm",
        detail: `confirm ${pool} (${windowName}) remains on track at the next check`,
      };
  }

  if (
    rate.perHour > 0 &&
    hoursToReset !== null &&
    verdict !== "stale" &&
    projectedLandingNextPeriod !== null &&
    projectedLandingNextPeriod < 100 - method.underPaceTolerance
  ) {
    notes.push(
      `next period strands ${round(100 - projectedLandingNextPeriod, 0)} points at the measured rate — plan work for the period after this reset`,
    );
    if (verdict === "on_track" && audible) {
      audible = {
        ...audible,
        action: "plan-next-period",
        detail:
          `${audible.detail}; also plan next-period work — at the measured rate the following period ` +
          `lands at ${round(projectedLandingNextPeriod, 0)}%`,
      };
    }
  }
  if (
    rate.perHour > 0 &&
    hoursToReset !== null &&
    verdict !== "stale" &&
    projectedLandingNextPeriod !== null &&
    projectedLandingNextPeriod > 100 + method.overPaceTolerance
  ) {
    notes.push(
      `next period walls at ${round(projectedLandingNextPeriod, 0)}% at the measured rate — cascade falls through early in the period after this reset`,
    );
  }

  return {
    pool,
    provider,
    plan: latest.plan,
    window: windowName,
    windowSeconds: seconds,
    currentPercent: round(currentPercent, 2),
    measuredAt: new Date(measuredAt).toISOString(),
    resetsAt,
    hoursToReset: hoursToReset === null ? null : round(hoursToReset, 2),
    rate: { ...rate, perHour: round(rate.perHour, 4), spanHours: round(rate.spanHours, 2) },
    requiredPerHour: requiredPerHour === null ? null : round(requiredPerHour, 4),
    projectedLanding,
    projectedLandingNextPeriod,
    exhaustionAt,
    verdict,
    reason,
    cascade,
    spillPctPoints,
    audible,
    notes,
  };
}

/**
 * Collapse a pool's ledger rows to one reading per tick.
 *
 * A multi-credential pool writes several rows under the same pool id for
 * one tick (openai.main does: two weekly credentials, each with its own
 * reset instant). The ledger payload carries no credential label, so the
 * rows cannot be told apart by identity — only by their numbers. The plan
 * therefore keeps the binding read per tick: the row with the highest
 * percentage for the pool's window, i.e. the credential closest to
 * walling (never the comfortable one). `simultaneous` reports how many
 * rows per tick the pool writes so the plan can say it did this.
 */
export function collapseCredentials(
  samples: RawSample[],
  asOf: number,
): { samples: RawSample[]; simultaneous: number; window: string | null } {
  const usable = samples
    .filter((s) => Date.parse(s.ts) <= asOf)
    .sort((a, b) => Date.parse(a.ts) - Date.parse(b.ts));
  if (!usable.length) return { samples: [], simultaneous: 0, window: null };

  // The pool's window is the one its readings most often name.
  const counts = new Map<string, { n: number; seconds: number }>();
  for (const s of usable) {
    const w = pickLargestWindow(s.windows);
    if (!w) continue;
    const hit = counts.get(w) ?? { n: 0, seconds: windowSeconds(w) ?? 0 };
    hit.n += 1;
    counts.set(w, hit);
  }
  let window: string | null = null;
  let bestScore = -1;
  for (const [name, c] of counts) {
    const score = c.n * 1_000_000 + c.seconds;
    if (score > bestScore) {
      bestScore = score;
      window = name;
    }
  }
  if (!window) return { samples: [], simultaneous: 0, window: null };

  // One bucket per tick (minute precision: credentials of one tick land
  // within a few seconds of each other).
  const buckets = new Map<number, RawSample[]>();
  for (const s of usable) {
    if (!s.windows.some((w) => w.name === window && w.percent !== null)) continue;
    const minute = Math.floor(Date.parse(s.ts) / 60_000);
    const list = buckets.get(minute) ?? [];
    list.push(s);
    buckets.set(minute, list);
  }

  let simultaneous = 0;
  const collapsed: RawSample[] = [];
  for (const minute of [...buckets.keys()].sort((a, b) => a - b)) {
    const rows = buckets.get(minute)!;
    // The gatherer can write the same reading twice in one tick (listing
    // plus explicit ask). Only genuinely different readings are different
    // credentials, so count distinct (percent, reset) pairs, not rows.
    const distinct = new Set(
      rows.map((s) => {
        const w = s.windows.find((x) => x.name === window)!;
        return `${Math.round((w.percent ?? -1) * 100)}|${w.resetsAt ?? ""}`;
      }),
    );
    if (distinct.size > simultaneous) simultaneous = distinct.size;
    const binding = [...rows].sort((a, b) => {
      const pa = a.windows.find((w) => w.name === window)!.percent ?? -1;
      const pb = b.windows.find((w) => w.name === window)!.percent ?? -1;
      return pb - pa || Date.parse(b.ts) - Date.parse(a.ts);
    })[0];
    collapsed.push(binding);
  }
  return { samples: collapsed, simultaneous, window };
}

/** Build the whole plan from decoded ledger samples. */
export function buildPlan(
  samples: RawSample[],
  asOfIso: string,
  opts: ProjectOptions & { generatedAt?: string; source?: string } = {},
): FullUtilizationPlan {
  const method: PlanMethod = { ...DEFAULT_METHOD, ...(opts.method ?? {}) };
  const asOf = Date.parse(asOfIso);
  const byPool = new Map<string, RawSample[]>();
  for (const s of samples) {
    const list = byPool.get(s.pool) ?? [];
    list.push(s);
    byPool.set(s.pool, list);
  }

  const pools: PoolProjection[] = [];
  const resourcePools: FullUtilizationPlan["resourcePools"] = [];
  const skip = new Set([
    ...(opts.skipPools ?? []),
    ...(opts.includeNonPools ? [] : [...NON_POOL_SERVICE_IDS].filter((id) => !RESOURCE_POOLS[id])),
    ...LEGACY_SERVICE_IDS,
  ]);
  for (const [pool, list] of [...byPool.entries()].sort((a, b) => a[0].localeCompare(b[0]))) {
    if (RESOURCE_POOLS[pool]) {
      const latest = [...list].sort((a, b) => Date.parse(a.ts) - Date.parse(b.ts)).pop()!;
      resourcePools.push({
        pool,
        provider: latest.provider,
        windows: latest.windows.map((w) => w.name),
        note: RESOURCE_POOLS[pool],
      });
      continue;
    }
    if (skip.has(pool)) continue;

    // A pool id can carry more than one quota window (a multi-credential
    // pool: openai.main's credentials reset on different weeks). Plan each
    // window instance rather than letting the newest row win at random.
    const collapsed = collapseCredentials(list, asOf);
    if (!collapsed.samples.length) continue;
    const projection = projectPool(pool, collapsed.samples, asOf, opts);
    if (!projection) continue;
    if (collapsed.simultaneous > 1) {
      projection.notes.push(
        `${collapsed.simultaneous} simultaneous ${collapsed.window} readings per tick (multi-credential pool) — ` +
          "the plan tracks the most-constrained credential",
      );
    }
    pools.push(projection);
  }

  const dataQuality: string[] = [];
  for (const p of pools) {
    if (p.verdict === "stale") {
      dataQuality.push(`${p.pool}: stale reading (${p.measuredAt}) — excluded from the burn plan until repaired`);
    }
    if (p.verdict === "no_reset") {
      dataQuality.push(`${p.pool}: ${p.reason}`);
    }
    if (p.verdict === "no_data") {
      dataQuality.push(`${p.pool}: no usable reading`);
    }
  }

  const count = (v: Verdict) => pools.filter((p) => p.verdict === v).length;
  return {
    version: 1,
    generatedAt: opts.generatedAt ?? new Date().toISOString(),
    asOf: new Date(asOf).toISOString(),
    method,
    pools,
    resourcePools,
    dataQuality,
    summary: {
      underPace: count("under_pace"),
      overPace: count("over_pace"),
      exhausted: count("exhausted"),
      onTrack: count("on_track"),
      stale: count("stale"),
      noReset: count("no_reset"),
      noData: count("no_data"),
    },
  };
}

/** Stable plan text. Tables sort by verdict weight, then pool id. */
export function renderPlanMarkdown(plan: FullUtilizationPlan, opts: { headingLevel?: number } = {}): string {
  const h = "#".repeat(opts.headingLevel ?? 2);
  const order: Record<Verdict, number> = {
    under_pace: 0,
    over_pace: 1,
    exhausted: 2,
    on_track: 3,
    no_reset: 4,
    stale: 5,
    no_data: 6,
  };
  const pools = [...plan.pools].sort(
    (a, b) => order[a.verdict] - order[b.verdict] || a.pool.localeCompare(b.pool),
  );
  const line = (p: PoolProjection): string =>
    `| \`${p.pool}\` | ${p.window} | ${p.currentPercent}% | ${p.resetsAt ?? "—"} | ` +
    `${p.hoursToReset === null ? "—" : p.hoursToReset.toFixed(1) + "h"} | ` +
    `${p.rate.confidence === "none" ? 0 : (p.rate.perHour * 24).toFixed(2)} pts/day | ` +
    `${p.requiredPerHour === null ? "—" : (p.requiredPerHour * 24).toFixed(2) + " pts/day"} | ` +
    `${p.projectedLanding === null ? "—" : p.projectedLanding.toFixed(0) + "%"} | ` +
    `${p.projectedLandingNextPeriod === null ? "—" : p.projectedLandingNextPeriod.toFixed(0) + "%"} | ` +
    `**${p.verdict}** | ` +
    `${p.cascade.group} ${p.cascade.position}/${p.cascade.of} → ${p.cascade.next ?? "—"} | ` +
    `${p.spillPctPoints ? p.spillPctPoints.toFixed(0) + " pts" : "—"}`;

  const header =
    "| Pool | Largest window | Now | Reset (UTC) | Left | Measured burn | Required burn | Landing @ reset | Landing @ next period | Verdict | Cascade | Spill |\n" +
    "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |";
  const table = [header, ...pools.map(line)].join("\n");

  const audibleLines = pools
    .filter((p) => p.audible && p.audible.action !== "confirm")
    .map(
      (p) =>
        `- **${p.audible!.at}** — \`${p.audible!.action}\` **${p.pool}** (${p.window}): ${p.audible!.detail}` +
        (p.audible!.deadline ? ` _(last useful action: ${p.audible!.deadline})_` : ""),
    );

  const notes = pools
    .filter((p) => p.notes.length)
    .map((p) => `- \`${p.pool}\`: ${p.notes.join("; ")}`);

  const resources = plan.resourcePools.map(
    (r) => `- \`${r.pool}\` (${r.provider}): ${r.windows.join(", ")} — ${r.note}`,
  );

  const parts = [
    `${h} Burn plan as of ${plan.asOf}`,
    "",
    `Computed ${plan.generatedAt} from the recorded quota ledger (source: \`opencode-quota\`, ` +
      `Turso \`fleet-platform.agent_quota_records\`), rate measured as net burn over the current ` +
      `window segment (capped at ${plan.method.rateWindowHours}h; never assumed). ` +
      `Judgement band: on track within ±${plan.method.overPaceTolerance}/${plan.method.underPaceTolerance} ` +
      `points of a 100% landing; readings stale after ${plan.method.staleHours}h. ` +
      `Check cadence: every ${plan.method.checkIntervalHours}h.`,
    "",
    `**Summary:** ${plan.summary.underPace} under pace · ${plan.summary.overPace} over pace · ` +
      `${plan.summary.exhausted} fully used · ${plan.summary.onTrack} on track · ` +
      `${plan.summary.stale} stale · ${plan.summary.noReset} no reset instant · ${plan.summary.noData} no data`,
    "",
    table,
    "",
    `${h} Dated audibles`,
    "",
    audibleLines.length ? audibleLines.join("\n") : "- none — every pool is on track",
    "",
    `${h} Data quality`,
    "",
    plan.dataQuality.length ? plan.dataQuality.map((d) => `- ${d}`).join("\n") : "- all readings fresh and projectable",
  ];
  if (notes.length) parts.push("", `${h} Projection notes`, "", notes.join("\n"));
  if (resources.length) parts.push("", `${h} Resource pools (not judged)`, "", resources.join("\n"));
  return parts.join("\n") + "\n";
}

/** A plan persisted to disk (the diff baseline). */
export interface PlanFile extends FullUtilizationPlan {}

export interface PlanDiff {
  /** Pools whose verdict changed since the previous plan. */
  verdictChanges: { pool: string; from: Verdict; to: Verdict }[];
  /** Pools whose projected landing moved by more than `thresholdPts`. */
  landingDrift: { pool: string; from: number | null; to: number | null; delta: number }[];
  /** Pools that need escalation now (newly under/over pace, or newly unreadable). */
  escalations: { pool: string; verdict: Verdict; detail: string }[];
  /** Pools that were flagged for action before and are now on track. */
  recovered: string[];
  /** True when the plan itself is absent (first run). */
  firstRun: boolean;
  /** Pools in the plan that were not in the baseline. */
  added: string[];
  /** Pools in the baseline that are no longer reported. */
  removed: string[];
}

/** Compare a check's plan against the standing plan (the diff the check reports). */
export function diffPlans(previous: PlanFile | null, current: PlanFile, thresholdPts = 5): PlanDiff {
  if (!previous) {
    return { verdictChanges: [], landingDrift: [], escalations: [], recovered: [], firstRun: true, added: current.pools.map((p) => p.pool), removed: [] };
  }
  const prevByPool = new Map(previous.pools.map((p) => [p.pool, p]));
  const curByPool = new Map(current.pools.map((p) => [p.pool, p]));
  const verdictChanges: PlanDiff["verdictChanges"] = [];
  const landingDrift: PlanDiff["landingDrift"] = [];
  const escalations: PlanDiff["escalations"] = [];
  const recovered: string[] = [];

  for (const [key, cur] of curByPool) {
    const pool = cur.pool;
    const prev = prevByPool.get(key);
    if (!prev) continue;
    if (prev.verdict !== cur.verdict) verdictChanges.push({ pool, from: prev.verdict, to: cur.verdict });
    if (prev.projectedLanding !== null && cur.projectedLanding !== null) {
      const delta = cur.projectedLanding - prev.projectedLanding;
      if (Math.abs(delta) >= thresholdPts) {
        landingDrift.push({ pool, from: prev.projectedLanding, to: cur.projectedLanding, delta: Math.round(delta * 10) / 10 });
      }
    }
    // "Fully used" is a completed period, not an escalation: the pool
    // burned what it had, and the cascade covers its work until reset.
    const actionable = (v: Verdict) => v === "under_pace" || v === "over_pace";
    if (actionable(cur.verdict) && !actionable(prev.verdict)) {
      escalations.push({ pool, verdict: cur.verdict, detail: cur.reason });
    }
    // Cleared = the row no longer asks for an action. A row that moved
    // from a verdict to "needs a reading repaired" has NOT cleared.
    if ((cur.verdict === "on_track" || cur.verdict === "exhausted") && actionable(prev.verdict)) {
      recovered.push(pool);
    }
  }

  const added = [...curByPool.keys()].filter((p) => !prevByPool.has(p));
  const removed = [...prevByPool.keys()].filter((p) => !curByPool.has(p));
  return { verdictChanges, landingDrift, escalations, recovered, firstRun: false, added, removed };
}

/** Is a check due, given the newest recorded check instant? */
export function checkDue(lastCheckIso: string | null, nowMs: number, intervalHours: number): boolean {
  if (!lastCheckIso) return true;
  const last = Date.parse(lastCheckIso);
  if (Number.isNaN(last)) return true;
  return (nowMs - last) / 3_600_000 >= intervalHours;
}

/** One recorded check run. Add-only: each run writes its own record. */
export interface CheckRecord {
  version: 1;
  /** How the check was produced: a live run, or a replay over recorded history. */
  mode: "live" | "replay";
  checkAt: string;
  asOf: string;
  planGeneratedAt: string;
  verdict: "confirm" | "escalate" | "repair";
  summary: FullUtilizationPlan["summary"];
  escalations: PlanDiff["escalations"];
  verdictChanges: PlanDiff["verdictChanges"];
  landingDrift: PlanDiff["landingDrift"];
  recovered: string[];
  audibleActions: { pool: string; at: string; action: string; detail: string }[];
  firstRun: boolean;
  added: string[];
  removed: string[];
}
