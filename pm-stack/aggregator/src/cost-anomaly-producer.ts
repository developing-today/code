// #4825 producer slice: assemble CostEvidence for the dormant detector
// (cost-anomaly.ts) from the spend rollup store (#4823). Before this
// module, postCostAnomalyTick's loadEvidence seam had no implementation
// at all — the detector could only ever be fed by test fixtures.
//
// What this module does:
// - Reads rollup records through an injected reader (production passes
//   SpendRollupStore.readAll; tests pass fixtures) and reduces them with
//   selectRollups, so one consistent attribution version per window is
//   paired — never a mix of an old and a restated version.
// - Pairs store records into evidence for the two store-only shapes:
//   burn-without-progress (every project/epic rollup) and
//   single-session-dominance (every epic token rollup), and pairs
//   adjacent unattributed windows for unattributed-growth.
// - Charge-without-activity evidence comes from an injected monthly
//   matched-charge totals source (the charges pipeline's output, per
//   spend-dollars.ts rule (1): this module never builds a money path).
//
// What it deliberately does not do: certify auxiliary facts. Closure
// counts, busiest root sessions, source-population totals and service
// activity are injected seams returning null when the source cannot
// certify them, and null suppresses the anomaly in the detector —
// partial data never fabricates an anomaly (#4825 rules). Implementing
// those certified sources, and any production activation of the tick,
// remain later slices (see COST-ANOMALY.md). This module performs no
// I/O of its own and is not called by the server.
import type { CostEvidence } from "./cost-anomaly.ts";
import { selectRollups, type SpendRollup } from "./spend-rollups.ts";

type WindowFields = Pick<SpendRollup, "windowKind" | "windowStart" | "windowEnd" | "coverage" | "attributionVersion">;
type ScopedWindow = WindowFields & Pick<SpendRollup, "scopeKind" | "scopeId">;

/** Certified child-closure count for one rollup's scope and window. */
export interface ClosedChildrenRecord extends ScopedWindow {
  count: number;
}

/** Certified busiest root-session token count for one epic rollup. */
export interface BusiestRootRecord extends ScopedWindow {
  pool: string;
  tokens: number;
}

/** Certified total source population for one rollup's window, before
 * attribution — never a sum of overlapping project/epic rows. */
export interface SourceTotalRecord extends WindowFields {
  unit: SpendRollup["unit"];
  pool: string | null;
  value: number;
}

/** One service's deduplicated matched PAYG total for one month. */
export interface MonthlyChargeTotal {
  service: string;
  /** Calendar month, YYYY-MM. */
  month: string;
  totalUsd: number;
  coverage: SpendRollup["coverage"];
}

/** Certified session/quota activity for one service in one month. */
export interface ServiceActivityRecord {
  service: string;
  /** Calendar month, YYYY-MM. */
  month: string;
  sessions: number;
  quotaActivity: number;
  coverage: SpendRollup["coverage"];
}

export interface CostAnomalyProducerDeps {
  /** Rollup store reader, e.g. `() => store.readAll()`. */
  readRollups: () => Promise<readonly SpendRollup[]>;
  /** Certified closure counts; absent/null = uncertified, suppresses. */
  closedChildren?: (spend: SpendRollup) => Promise<ClosedChildrenRecord | null> | ClosedChildrenRecord | null;
  /** Certified busiest root sessions; absent/null = uncertified. */
  busiestRoot?: (spend: SpendRollup) => Promise<BusiestRootRecord | null> | BusiestRootRecord | null;
  /** Certified source-population totals; absent/null = uncertified. */
  sourceTotal?: (rollup: SpendRollup) => Promise<SourceTotalRecord | null> | SourceTotalRecord | null;
  /** Monthly matched-charge totals from the charges pipeline. */
  monthlyCharges?: () => Promise<readonly MonthlyChargeTotal[]> | readonly MonthlyChargeTotal[];
  /** Certified per-service activity; absent/null = uncertified. */
  serviceActivity?: (
    charge: MonthlyChargeTotal,
  ) => Promise<ServiceActivityRecord | null> | ServiceActivityRecord | null;
}

/**
 * Assemble the full evidence batch for one anomaly tick. The output is
 * shaped for direct use as postCostAnomalyTick's `loadEvidence`. Order
 * is deterministic: rollups in selectRollups order (burn, then
 * dominance, per record), growth pairs in current-window order, then
 * charges in provider order.
 */
export async function loadCostAnomalyEvidence(deps: CostAnomalyProducerDeps): Promise<CostEvidence[]> {
  const rollups = selectRollups(await deps.readRollups());
  const evidence: CostEvidence[] = [];

  for (const spend of rollups) {
    if (spend.scopeKind === "unattributed") continue;
    evidence.push({
      kind: "burn-without-progress",
      spend,
      closedChildren: deps.closedChildren ? await deps.closedChildren(spend) : null,
    });
    if (spend.scopeKind === "epic" && spend.unit === "tokens") {
      evidence.push({
        kind: "single-session-dominance",
        spend,
        busiestRoot: deps.busiestRoot ? await deps.busiestRoot(spend) : null,
      });
    }
  }

  // Unattributed growth pairs each unattributed token rollup with the
  // rollup of the immediately preceding window (same kind, scope and
  // pool, ending exactly where the current one starts). A missing
  // predecessor emits no evidence at all: there is no comparison.
  const unattributed = rollups.filter((r) => r.scopeKind === "unattributed" && r.unit === "tokens");
  const byWindowEnd = new Map<string, SpendRollup>();
  for (const r of unattributed) byWindowEnd.set([r.windowKind, r.windowEnd, r.scopeId, r.pool].join("|"), r);
  for (const current of unattributed) {
    const previous = byWindowEnd.get(
      [current.windowKind, current.windowStart, current.scopeId, current.pool].join("|"),
    );
    if (!previous) continue;
    evidence.push({
      kind: "unattributed-growth",
      current,
      previous,
      currentTotal: deps.sourceTotal ? await deps.sourceTotal(current) : null,
      previousTotal: deps.sourceTotal ? await deps.sourceTotal(previous) : null,
    });
  }

  if (deps.monthlyCharges) {
    for (const charges of await deps.monthlyCharges()) {
      evidence.push({
        kind: "charge-without-activity",
        charges,
        activity: deps.serviceActivity ? await deps.serviceActivity(charges) : null,
      });
    }
  }

  return evidence;
}
