// Analytics + projection math for the /quota "Analytics & projection"
// section (Drew's order, 2026-11-13). Pure functions over plain
// values — no I/O, no clock reads except through arguments. The data
// layer is analytics-store.ts; rendering is quota-analytics-view.ts.
//
// Honesty rules (mirroring burn-history.ts / quota-exhaustion.ts):
// a projection that cannot be measured is an explicit state with a
// reason — never an invented date, never "safe".

/** One dated percent reading of one pool window. */
export interface PctPoint {
  /** Observation instant, ms epoch. */
  ts: number;
  /** Percent of the window's limit used at that instant, 0..100+. */
  pct: number;
  /** The window's reset instant as reported with that reading; two
   * readings belong to the same reset period only when their resets
   * agree within PROJECTION_PERIOD_TOLERANCE_MS (gatherers jitter
   * the reported reset by seconds between reads — string equality
   * would split one period into hundreds). */
  resetsAt: string | null;
}

export type ProjectionState =
  /** Percent is already at/over the limit. */
  | "walled"
  /** Measured burn crosses 100% before the reset. */
  | "projected"
  /** Measured burn does not cross 100% before the reset (or the
   * window has no reset to cross before). */
  | "no-wall-before-reset"
  /** Not enough same-period readings to measure a rate. */
  | "insufficient-history";

export interface WindowProjection {
  state: ProjectionState;
  /** Measured burn in percentage points per hour: the least-squares
   * slope of the same-period readings inside the fit window. Null
   * when no rate was measured. */
  ratePtsPerHour: number | null;
  /** Same-period readings behind the rate. */
  samples: number;
  /** Projected instant the window reaches 100%, when a positive rate
   * was measured (may fall after the reset — the caller decides how
   * to present that; the state already says so). */
  projectedWallAt: number | null;
  /** now -> projected wall in ms; 0 when already walled; null when
   * no positive rate was measured. */
  durationUntilWallMs: number | null;
  /** Whether the projected wall lands before the reset, when both
   * exist. True when already walled. */
  wallsBeforeReset: boolean | null;
  /** Why no projection exists, when state is "insufficient-history"
   * or the rate is flat/falling. */
  reason?: string;
}

/** Minimum span of readings before a slope means anything. */
export const PROJECTION_MIN_SPAN_MS = 10 * 60_000;
/** Two reset instants closer than this are the same period: the
 * shortest real window is 5h, so adjacent periods always differ by
 * at least that, while one period's reported reset jitters by
 * seconds. */
export const PROJECTION_PERIOD_TOLERANCE_MS = 2 * 60 * 60_000;

function samePeriod(pointResetsAt: string | null, liveResetsAt: string | null): boolean {
  if (liveResetsAt == null) return true;
  if (pointResetsAt == null) return false;
  const a = Date.parse(pointResetsAt);
  const b = Date.parse(liveResetsAt);
  if (!Number.isFinite(a) || !Number.isFinite(b)) return pointResetsAt === liveResetsAt;
  return Math.abs(a - b) <= PROJECTION_PERIOD_TOLERANCE_MS;
}
/** Only readings inside this trailing window join a fit: older burn
 * says little about the current pace, and the same-period filter
 * bounds the fit to the live reset period anyway. */
export const PROJECTION_FIT_WINDOW_MS = 24 * 60 * 60_000;

/**
 * Project one pool window from its percent-reading series.
 *
 * The fit uses only readings from the CURRENT reset period (same
 * resetsAt as the live window) inside the trailing fit window — a
 * pair straddling a reset measures the reset, not the burn.
 */
export function projectWindow(args: {
  points: PctPoint[];
  pctNow: number;
  resetsAt: string | null;
  now: number;
}): WindowProjection {
  const { pctNow, resetsAt, now } = args;
  if (pctNow >= 100) {
    return {
      state: "walled",
      ratePtsPerHour: null,
      samples: 0,
      projectedWallAt: null,
      durationUntilWallMs: 0,
      wallsBeforeReset: true,
    };
  }
  const period = args.points
    .filter(
      (p) =>
        p.ts <= now &&
        now - p.ts <= PROJECTION_FIT_WINDOW_MS &&
        samePeriod(p.resetsAt, resetsAt) &&
        Number.isFinite(p.pct),
    )
    .sort((a, b) => a.ts - b.ts);
  if (period.length < 2 || period[period.length - 1].ts - period[0].ts < PROJECTION_MIN_SPAN_MS) {
    return {
      state: "insufficient-history",
      ratePtsPerHour: null,
      samples: period.length,
      projectedWallAt: null,
      durationUntilWallMs: null,
      wallsBeforeReset: null,
      reason: "fewer than two same-period readings spanning 10 minutes",
    };
  }
  const n = period.length;
  const meanTs = period.reduce((s, p) => s + p.ts, 0) / n;
  const meanPct = period.reduce((s, p) => s + p.pct, 0) / n;
  let num = 0;
  let den = 0;
  for (const p of period) {
    num += (p.ts - meanTs) * (p.pct - meanPct);
    den += (p.ts - meanTs) ** 2;
  }
  const ratePtsPerHour = den > 0 ? (num / den) * 3_600_000 : 0;
  if (!(ratePtsPerHour > 0)) {
    return {
      state: "no-wall-before-reset",
      ratePtsPerHour,
      samples: n,
      projectedWallAt: null,
      durationUntilWallMs: null,
      wallsBeforeReset: false,
      reason: "measured burn is flat or falling",
    };
  }
  const wallAt = now + ((100 - pctNow) / ratePtsPerHour) * 3_600_000;
  const resetMs = resetsAt ? Date.parse(resetsAt) : NaN;
  const wallsBeforeReset = Number.isFinite(resetMs) ? wallAt < resetMs : null;
  return {
    state: wallsBeforeReset === false ? "no-wall-before-reset" : "projected",
    ratePtsPerHour,
    samples: n,
    projectedWallAt: wallAt,
    durationUntilWallMs: wallAt - now,
    wallsBeforeReset,
  };
}

/** Absolute + percent difference of a current value vs a previous
 * (e.g. same window last week). pct is null when the previous value
 * is 0 — an honest absence, not Infinity. */
export interface WowDelta {
  abs: number;
  pct: number | null;
}

export function wowDelta(current: number, previous: number): WowDelta {
  const abs = current - previous;
  return { abs, pct: previous !== 0 ? (abs / Math.abs(previous)) * 100 : null };
}

/** Token totals as recorded by the route proxy (route_attempt):
 * `input` excludes cache reads/writes where the provider reports
 * them separately — the columns are presented as recorded. */
export interface TokenTotals {
  input: number;
  output: number;
  reasoning: number;
  cacheRead: number;
  cacheWrite: number;
}

export const EMPTY_TOKENS: TokenTotals = { input: 0, output: 0, reasoning: 0, cacheRead: 0, cacheWrite: 0 };

export function totalTokens(t: TokenTotals): number {
  return t.input + t.output + t.reasoning + t.cacheRead + t.cacheWrite;
}

export interface TokenRatios {
  /** Uncached input tokens per output token; null with no output. */
  inPerOut: number | null;
  /** Cache reads as a percent of all input-side tokens
   * (input + cacheRead + cacheWrite); null with no input side. */
  cacheSharePct: number | null;
  total: number;
}

export function tokenRatios(t: TokenTotals): TokenRatios {
  const inputSide = t.input + t.cacheRead + t.cacheWrite;
  return {
    inPerOut: t.output > 0 ? t.input / t.output : null,
    cacheSharePct: inputSide > 0 ? (t.cacheRead / inputSide) * 100 : null,
    total: totalTokens(t),
  };
}

/** One quota window as the unlock picture needs it. */
export interface WindowLike {
  name: string;
  percent: number;
  resetsAt: string | null;
}

export interface UnlockPicture {
  /** The binding (gating) window's reset — when the pool unlocks. */
  bindingResetsAt: string | null;
  /** Names of windows that free up at that same reset. */
  freesAtReset: string[];
  /** Windows still at/over 100% after that reset (later or no
   * reset): what stays walled when the binding window unlocks. */
  stillWalledAfter: { name: string; resetsAt: string | null }[];
}

/**
 * The unlock picture for one pool, relative to its binding window
 * (the window that gates the pool — the caller picks it, normally
 * the fullest or the one projected to wall first): what frees when
 * the binding window resets, and what stays walled past it.
 */
export function unlockPicture(windows: WindowLike[], binding: WindowLike): UnlockPicture {
  const bindingResetsAt = binding.resetsAt;
  const freesAtReset = windows.filter((w) => w.resetsAt != null && w.resetsAt === bindingResetsAt).map((w) => w.name);
  const bindingMs = bindingResetsAt ? Date.parse(bindingResetsAt) : NaN;
  const stillWalledAfter = windows
    .filter((w) => {
      if (w === binding || w.percent < 100) return false;
      if (w.resetsAt == null) return true;
      if (!Number.isFinite(bindingMs)) return false;
      return Date.parse(w.resetsAt) > bindingMs;
    })
    .map((w) => ({ name: w.name, resetsAt: w.resetsAt }));
  return { bindingResetsAt, freesAtReset, stillWalledAfter };
}

/** Compact duration for key-value rows: "4d 8h", "5h 12m", "42m". */
export function fmtDuration(ms: number | null): string {
  if (ms == null || !Number.isFinite(ms)) return "—";
  if (ms <= 0) return "now";
  const minutes = Math.round(ms / 60_000);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  const days = Math.floor(hours / 24);
  if (days > 0) return `${days}d ${hours % 24}h`;
  return `${hours}h ${minutes % 60}m`;
}

/**
 * US federal holidays (the observed federal calendar), keyed by
 * local ISO day. Only the years the route-attempt history covers
 * are tabled; an untabled day is simply unannotated, never guessed.
 */
const US_FEDERAL_HOLIDAYS: Record<string, string> = {
  "2025-10-13": "Columbus Day",
  "2025-11-11": "Veterans Day",
  "2025-11-27": "Thanksgiving",
  "2025-12-25": "Christmas",
  "2026-01-01": "New Year's Day",
  "2026-01-19": "MLK Day",
  "2026-02-16": "Presidents Day",
  "2026-05-25": "Memorial Day",
  "2026-06-19": "Juneteenth",
  "2026-07-03": "Independence Day (observed)",
  "2026-09-07": "Labor Day",
  "2026-10-12": "Columbus Day",
  "2026-11-11": "Veterans Day",
  "2026-11-26": "Thanksgiving",
  "2026-12-25": "Christmas",
  "2027-01-01": "New Year's Day",
  "2027-01-18": "MLK Day",
};

export function usFederalHoliday(isoDay: string): string | null {
  return US_FEDERAL_HOLIDAYS[isoDay] ?? null;
}
