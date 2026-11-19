// #4825 detector/intake slice. No server, scheduler, or producer calls it.
import type { AlertsStore, NewIncident } from "./alerts.ts";
import type { MatchedCharge } from "./spend-dollars.ts";
import { validateSpendRollup, windowRange, type SpendRollup } from "./spend-rollups.ts";

const THRESHOLDS = {
  minimumSpend: 0,
  closedChildren: 0,
  dominancePct: 50,
  unattributedDeltaPoints: 10,
  minimumChargeUsd: 0,
  activityCount: 0,
} as const;
type Window = Pick<SpendRollup, "windowKind" | "windowStart" | "windowEnd" | "coverage" | "attributionVersion">;
type Scoped = Window & Pick<SpendRollup, "scopeKind" | "scopeId">;
type CompleteTotal = Window & Pick<SpendRollup, "unit" | "pool"> & { value: number };
type CostEvidence =
  | { kind: "burn-without-progress"; spend: SpendRollup; closedChildren: (Scoped & { count: number }) | null }
  | {
      kind: "single-session-dominance";
      spend: SpendRollup;
      busiestRoot: (Scoped & { pool: string; tokens: number }) | null;
    }
  | {
      kind: "unattributed-growth";
      current: SpendRollup;
      previous: SpendRollup;
      // Same full source population as the numerator, before attribution
      // and without summing overlapping per-project/per-epic rollups.
      currentTotal: CompleteTotal | null;
      previousTotal: CompleteTotal | null;
    }
  | {
      kind: "charge-without-activity";
      // Deduplicated monthly total from matched PAYG charges only.
      charges: Pick<MatchedCharge, "service"> & { month: string; totalUsd: number; coverage: Window["coverage"] };
      activity: {
        service: string;
        month: string;
        sessions: number;
        quotaActivity: number;
        coverage: Window["coverage"];
      } | null;
    };

export type { CostEvidence };

function completeSpend(spend: SpendRollup, now: Date): boolean {
  try {
    validateSpendRollup(spend);
    const range = windowRange(spend.windowKind, spend.windowStart);
    return (
      spend.coverage === "complete" &&
      range.start === spend.windowStart &&
      range.end === spend.windowEnd &&
      Date.parse(spend.coverageStart) <= Date.parse(range.start) &&
      Date.parse(range.end) <= now.getTime()
    );
  } catch {
    return false;
  }
}

function sameWindow(aux: Window, spend: SpendRollup): boolean {
  return (
    aux.coverage === "complete" &&
    aux.windowKind === spend.windowKind &&
    aux.windowStart === spend.windowStart &&
    aux.windowEnd === spend.windowEnd &&
    aux.attributionVersion === spend.attributionVersion
  );
}

function sameScope(aux: Scoped, spend: SpendRollup): boolean {
  return sameWindow(aux, spend) && aux.scopeKind === spend.scopeKind && aux.scopeId === spend.scopeId;
}

function warning(
  kind: CostEvidence["kind"],
  scope: string,
  window: [string, string],
  measure: [string, string | null],
  evidence: string,
  link: string,
): NewIncident {
  return {
    source: "cost-anomaly",
    key: JSON.stringify([kind, scope, ...window, ...measure]),
    severity: "warning",
    title: `${kind.replaceAll("-", " ")}: ${scope}`,
    body: `${scope} · [${window[0]}, ${window[1]}) UTC · ${evidence} · ${link}`,
  };
}

function spendWarning(kind: CostEvidence["kind"], spend: SpendRollup, evidence: string): NewIncident {
  return warning(
    kind,
    `${spend.scopeKind}:${spend.scopeId}`,
    [spend.windowStart, spend.windowEnd],
    [spend.unit, spend.pool],
    spend.pool ? `${evidence} (pool: ${spend.pool})` : evidence,
    "/projects",
  );
}

function share(numerator: SpendRollup, denominator: CompleteTotal | null): number | null {
  if (
    !denominator ||
    !sameWindow(denominator, numerator) ||
    denominator.unit !== numerator.unit ||
    denominator.pool !== numerator.pool ||
    !Number.isFinite(denominator.value) ||
    denominator.value <= 0 ||
    denominator.value < numerator.value
  )
    return null;
  return 100 * (numerator.value / denominator.value);
}

const pct = (n: number) => Number(n.toFixed(4));

export function detectCostAnomaly(evidence: CostEvidence, now: Date): NewIncident | null {
  switch (evidence.kind) {
    case "burn-without-progress": {
      const { spend, closedChildren } = evidence;
      if (
        !completeSpend(spend, now) ||
        spend.scopeKind === "unattributed" ||
        spend.value <= THRESHOLDS.minimumSpend ||
        !closedChildren ||
        !sameScope(closedChildren, spend) ||
        closedChildren.count !== THRESHOLDS.closedChildren
      )
        return null;
      const unit = spend.unit === "usd" ? "USD" : spend.unit === "share_pct" ? "% of plan window" : "tokens";
      return spendWarning(evidence.kind, spend, `${spend.value} ${unit} consumed; 0 child closures`);
    }
    case "single-session-dominance": {
      const { spend, busiestRoot } = evidence;
      if (
        !completeSpend(spend, now) ||
        spend.scopeKind !== "epic" ||
        spend.unit !== "tokens" ||
        spend.value <= THRESHOLDS.minimumSpend ||
        !busiestRoot ||
        !sameScope(busiestRoot, spend) ||
        busiestRoot.pool !== spend.pool ||
        !Number.isFinite(busiestRoot.tokens) ||
        busiestRoot.tokens > spend.value
      )
        return null;
      const percent = (busiestRoot.tokens / spend.value) * 100;
      if (percent <= THRESHOLDS.dominancePct) return null;
      return spendWarning(
        evidence.kind,
        spend,
        `${busiestRoot.tokens} tokens in busiest root / ${spend.value} tokens total (${pct(percent)}%)`,
      );
    }
    case "unattributed-growth": {
      const { current, previous } = evidence;
      if (
        !completeSpend(current, now) ||
        !completeSpend(previous, now) ||
        current.scopeKind !== "unattributed" ||
        previous.scopeKind !== "unattributed" ||
        current.scopeId !== previous.scopeId ||
        current.unit !== "tokens" ||
        previous.unit !== current.unit ||
        current.pool !== previous.pool ||
        current.windowKind !== previous.windowKind ||
        current.attributionVersion !== previous.attributionVersion ||
        previous.windowEnd !== current.windowStart
      )
        return null;
      const before = share(previous, evidence.previousTotal);
      const after = share(current, evidence.currentTotal);
      // A mathematically exact boundary must not fire due to floating point noise.
      if (
        before === null ||
        after === null ||
        after - before <= THRESHOLDS.unattributedDeltaPoints + Number.EPSILON * 100
      )
        return null;
      return spendWarning(evidence.kind, current, `${pct(after)}% unattributed vs ${pct(before)}% in preceding window`);
    }
    case "charge-without-activity": {
      const { charges, activity } = evidence;
      if (
        !activity ||
        activity.coverage !== "complete" ||
        charges.coverage !== "complete" ||
        !charges.service.trim() ||
        activity.service !== charges.service ||
        !/^\d{4}-(0[1-9]|1[0-2])$/.test(charges.month) ||
        charges.month !== activity.month ||
        !Number.isFinite(charges.totalUsd) ||
        charges.totalUsd <= THRESHOLDS.minimumChargeUsd ||
        activity.sessions !== THRESHOLDS.activityCount ||
        activity.quotaActivity !== THRESHOLDS.activityCount
      )
        return null;
      const { start, end } = windowRange("month", charges.month + "-01");
      if (!(Date.parse(end) <= now.getTime())) return null;
      return warning(
        evidence.kind,
        `service:${charges.service}`,
        [start, end],
        ["usd", null],
        `matched PAYG USD ${charges.totalUsd}; 0 sessions / quota activity`,
        "/quota",
      );
    }
  }
}

/** Explicit injection seam for a future certified producer. No I/O when off. */
export async function postCostAnomalyTick(options: {
  enabled?: boolean;
  now: Date;
  loadEvidence: () => Promise<readonly CostEvidence[]>;
  store: Pick<AlertsStore, "postIncident">;
}): Promise<number> {
  if (options.enabled !== true) return 0;
  const warnings = new Map<string, NewIncident | null>();
  for (const evidence of await options.loadEvidence()) {
    const incident = detectCostAnomaly(evidence, options.now);
    if (!incident) continue;
    if (!warnings.has(incident.key)) warnings.set(incident.key, incident);
    else if (warnings.get(incident.key)?.body !== incident.body) warnings.set(incident.key, null);
  }
  let posted = 0;
  for (const incident of warnings.values()) {
    if (!incident) continue;
    await options.store.postIncident(incident);
    posted++;
  }
  return posted;
}
