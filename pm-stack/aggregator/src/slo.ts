/** Pure SLO budget calculations for the watchdog health surface (#9988).
 * The journal/sampler owns completeness; this module applies the design's
 * one-minute sample semantics without inventing values for age facets. */
export type SloSample = {
  atMs: number;
  bad: boolean | null;
  value: number | null;
};

export type SloDefinition = {
  id: string;
  label: string;
  target: string;
  budgetFraction: number;
  windowMs: number;
  missing: "bad" | "unknown";
  /** Bad samples beyond this threshold consume extra budget minutes. */
  doubleBadAbove?: number;
};

const DAY = 86_400_000;
const MINUTE = 60_000;
const THIRTY_DAYS = 30 * DAY;

export const SLO_DEFINITIONS: readonly SloDefinition[] = [
  {
    id: "feed-age",
    label: "Feed age",
    target: "p95 ≤ 10 min; none > 30 min",
    budgetFraction: 0.005,
    windowMs: 7 * DAY,
    missing: "unknown",
    doubleBadAbove: 30 * MINUTE,
  },
  {
    id: "quota-age",
    label: "Quota read age",
    target: "p99 ≤ 75 min",
    budgetFraction: 0.01,
    windowMs: THIRTY_DAYS,
    missing: "unknown",
  },
  {
    id: "orchestration-age",
    label: "Orchestration freshness",
    target: "p99 ≤ 90 min",
    budgetFraction: 0.01,
    windowMs: THIRTY_DAYS,
    missing: "unknown",
  },
  {
    id: "aggregator-up",
    label: "Aggregator",
    target: "99.5% of minutes",
    budgetFraction: 0.005,
    windowMs: THIRTY_DAYS,
    missing: "bad",
  },
  {
    id: "intake-up",
    label: "Intake",
    target: "99.5% of minutes",
    budgetFraction: 0.005,
    windowMs: THIRTY_DAYS,
    missing: "bad",
  },
  {
    id: "projector-up",
    label: "Projector",
    target: "99.5% of minutes",
    budgetFraction: 0.005,
    windowMs: THIRTY_DAYS,
    missing: "bad",
  },
  {
    id: "pad-grist-up",
    label: "Pad / Grist",
    target: "99% of minutes",
    budgetFraction: 0.01,
    windowMs: THIRTY_DAYS,
    missing: "bad",
  },
  {
    id: "tunnel-up",
    label: "Tunnel",
    target: "99.9% of minutes",
    budgetFraction: 0.001,
    windowMs: THIRTY_DAYS,
    missing: "bad",
  },
];

export type SloBudget = {
  currentValue: number | null;
  target: string;
  budgetRemainingPercent: number | null;
  burnRate1h: number | null;
  state: "ok" | "burning" | "exhausted" | "unknown";
  badMinutes: number;
  budgetMinutes: number;
  unknownSamples: number;
};

/** One sample per minute, the last one written for it (#14062): a minute
 * sampled twice (two writers, a restart inside the minute) is still one
 * minute of the budget, never two bad minutes. Sorted by time. */
function onePerMinute(samples: readonly SloSample[]): SloSample[] {
  const byMinute = new Map<number, SloSample>();
  for (const sample of samples) {
    const minute = Math.floor(sample.atMs / MINUTE) * MINUTE;
    const held = byMinute.get(minute);
    if (held === undefined || sample.atMs >= held.atMs) byMinute.set(minute, sample);
  }
  return [...byMinute.values()].sort((a, b) => a.atMs - b.atMs);
}

/** Compute budget from observations in the rolling window. A missing minute
 * is explicitly charged for up facets and excluded-but-counted for age facets. */
export function computeSloBudget(
  definition: SloDefinition,
  samples: readonly SloSample[],
  nowMs: number,
  observedSinceMs?: number,
): SloBudget {
  const start = nowMs - definition.windowMs;
  const inWindow = onePerMinute(samples.filter((s) => s.atMs > start && s.atMs <= nowMs));
  const windowStart = observedSinceMs === undefined ? start : Math.max(start, observedSinceMs - MINUTE);
  const firstExpected = Math.floor(windowStart / MINUTE) * MINUTE + MINUTE;
  const lastExpected = Math.floor(nowMs / MINUTE) * MINUTE;
  const expected = Math.max(0, Math.floor((lastExpected - firstExpected) / MINUTE) + 1);
  const seen = new Set(inWindow.map((s) => Math.floor(s.atMs / MINUTE) * MINUTE));
  let missing = Math.max(0, expected - seen.size);
  let unknownSamples = 0;
  let badMinutes = 0;
  const badLastHour = new Set<number>();
  for (const sample of inWindow) {
    const minute = Math.floor(sample.atMs / MINUTE) * MINUTE;
    if (sample.bad === null) {
      unknownSamples++;
      continue;
    }
    if (!sample.bad) continue;
    const weight =
      definition.doubleBadAbove !== undefined && sample.value !== null && sample.value > definition.doubleBadAbove
        ? 2
        : 1;
    badMinutes += weight;
    if (sample.atMs > nowMs - 60 * MINUTE) badLastHour.add(minute);
  }
  if (definition.missing === "bad") {
    badMinutes += missing;
    for (let at = firstExpected; at <= lastExpected; at += MINUTE) {
      if (!seen.has(at) && at > nowMs - 60 * MINUTE) badLastHour.add(at);
    }
  } else {
    unknownSamples += missing;
  }
  const budgetMinutes = Math.floor((definition.windowMs / MINUTE) * definition.budgetFraction);
  const remaining =
    definition.missing === "unknown" && expected > 0 && unknownSamples / expected > 0.5
      ? null
      : Math.max(0, 100 * (1 - badMinutes / budgetMinutes));
  const burnRate1h =
    budgetMinutes > 0 ? badLastHour.size / 60 / (budgetMinutes / (definition.windowMs / MINUTE)) : null;
  return {
    currentValue: inWindow.at(-1)?.value ?? null,
    target: definition.target,
    budgetRemainingPercent: remaining,
    burnRate1h,
    state: remaining === null ? "unknown" : remaining <= 0 ? "exhausted" : badLastHour.size > 0 ? "burning" : "ok",
    badMinutes,
    budgetMinutes,
    unknownSamples,
  };
}

export type SloFacetSummary = SloBudget & {
  id: string;
  label: string;
  /** False when no sample in the window carries a real signal (for example
   * a facet whose probe is not wired yet); such facets show no budget. */
  sourced: boolean;
  latestBad: boolean | null;
};

export function summarizeSloFacet(
  definition: SloDefinition,
  samples: readonly SloSample[],
  nowMs: number,
): SloFacetSummary {
  const inWindow = samples.filter((s) => s.atMs > nowMs - definition.windowMs && s.atMs <= nowMs);
  const sourced = inWindow.some((s) => s.bad !== null);
  const earliest = inWindow.reduce<number | undefined>(
    (min, s) => (min === undefined || s.atMs < min ? s.atMs : min),
    undefined,
  );
  const budget = computeSloBudget(definition, inWindow, nowMs, earliest);
  const latest = inWindow.reduce<SloSample | undefined>(
    (last, s) => (last === undefined || s.atMs > last.atMs ? s : last),
    undefined,
  );
  return {
    ...budget,
    id: definition.id,
    label: definition.label,
    sourced,
    latestBad: latest?.bad ?? null,
    budgetRemainingPercent: sourced ? budget.budgetRemainingPercent : null,
    // #14062: a facet with no signal source shows no burn either; a
    // burn figure beside "unknown" reads as a measurement it is not.
    burnRate1h: sourced ? budget.burnRate1h : null,
    state: sourced ? budget.state : "unknown",
  };
}
