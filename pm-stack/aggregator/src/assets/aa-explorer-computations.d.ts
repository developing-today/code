export function pctRank(
  values: Array<number | null | undefined>,
  value: number | null | undefined,
  higherBetter: boolean,
): number | null;
export function paretoSet<T>(
  rows: T[],
  dims: Array<{ get: (row: T) => number | null | undefined; better: "high" | "low" }>,
): T[];
export function gapMetrics<
  T extends { key: string; score?: number | null; cost?: number | null; time?: number | null; tps?: number | null },
>(
  rows: T[],
): Record<
  string,
  {
    score: number | null | undefined;
    cost: number | null | undefined;
    time: number | null | undefined;
    tps: number | null | undefined;
    gapScore: number | null;
    gapCost: number | null;
    gapSpeed: number | null;
  }
>;
export function applyInequalityFilters<T extends { key: string }>(
  rows: T[],
  mode: string,
  combine: string,
  conditions: Array<{ metric: string; op: string; value: string }>,
): T[];
export function logEdges(values: Array<number | null | undefined>, count: number): number[];
export function scoreEdges<T extends { score?: number | null }>(rows: T[], count: number): number[];
export function bandIndex(value: number, edges: number[]): number;
export function computeRecommendations<
  T extends { key: string; score?: number | null; cost?: number | null; time?: number | null; creator: string },
>(
  rows: T[],
  settings: Record<string, string | number>,
  bandCount: number | string,
  scoreName: string,
): {
  bands: Array<{
    members: T[];
    picks: Array<{ p: T; factor: number }>;
    decision: Record<string, string>;
    [key: string]: unknown;
  }>;
  empty: string | null;
};
export function normName(value: unknown): string;
export function baselineRatios<
  T extends { key: string; score?: number | null; cost?: number | null; time?: number | null },
>(
  rows: T[],
  baselineKeys: string[],
): Record<
  string,
  {
    scorePerCost: number | null;
    scorePerSpeed: number | null;
    vs: Record<string, { scorePerCost: number | null; scorePerSpeed: number | null }>;
  }
>;
export function sortRows<T extends { key: string }>(
  rows: T[],
  key: string | ((row: T) => number | string | null | undefined) | null,
  dir: "asc" | "desc",
): T[];
export function applyDominanceFilters<
  T extends { key: string; score?: number | null; cost?: number | null; time?: number | null },
>(rows: T[], opts: { dominated?: boolean; slower?: boolean; dumber?: boolean; pricier?: boolean }): T[];
export function unpricedScoredPoints<
  T extends { score?: number | null; cost?: number | null; family?: string; effort?: string | null },
>(points: T[]): T[];
export function costScoreOls<T extends { cost?: number | null; score?: number | null }>(
  points: T[],
): { slope: number; intercept: number; n: number; costMin: number; costMax: number } | null;
export function costScoreEnvelope<T extends { cost?: number | null; score?: number | null }>(
  points: T[],
): Array<{ cost: number; score: number }>;
export function costScoreTail<T extends { cost?: number | null; score?: number | null }>(
  points: T[],
  binCount?: number,
): Array<{ cost: number; score: number }>;
export function projectTask(
  model: any,
  settings: Record<string, number | string>,
  aaPoints: any[],
  caiPoints: any[],
): { cost: number | null; time: number | null; band: any };

export interface ChartViewState {
  base: number[];
  cur: number[] | null;
}
export function parseChartViewBox(s: string | null | undefined): number[] | null;
export function effectiveChartView(st: ChartViewState): number[];
export function clampChartView(st: ChartViewState): void;
export function zoomChartView(st: ChartViewState, factor: number, fx: number | null, fy: number | null): void;
export function panChartView(st: ChartViewState, dx: number, dy: number, width: number, height: number): boolean;
export function resetChartView(st: ChartViewState): void;
export function chartClickSuppressed(rawStamp: string | number | null | undefined, now: number): boolean;
export type LabelMode = "key" | "all" | "none";
export function resolveLabelMode(mode: unknown): LabelMode;
export function shouldLabelPoint(mode: unknown, onFrontier: boolean, isSelected: boolean): boolean;
export type ChartLimitMode = "unset" | "atmost" | "atleast";
export function resolveLimitMode(mode: unknown): ChartLimitMode;
export function limitThreshold(raw: unknown): number | null;
export function passesChartLimit(
  value: number | null | undefined,
  mode: unknown,
  threshold: unknown,
): boolean;
export function applyChartLimit<T>(
  rows: T[],
  get: (row: T) => number | null | undefined,
  spec: { mode?: unknown; value?: unknown } | null | undefined,
): T[];
export function capPointsPerBand<T>(rows: T[], maxPer: unknown, bandOf: (row: T) => string | number): T[];
export type ChartMagnify = "fit" | "150" | "200";
export function resolveMagnify(mag: unknown): ChartMagnify;
export function magnifyWidth(mag: unknown): string;
