// Deterministic C7 period rollups over canonical, persisted lower-grain records.
import { dailyReportPeriod, type ReportPeriod, type ReportPeriodKind } from "./report-periods.ts";
import type { ReportRecord, ReportRecordBlock, ReportRecordSource, ReportRecordLocation } from "./report-records.ts";
import { appendReportRecord, makeReportRecord, readCurrentReportRecords } from "./report-records.ts";

export const REPORT_TEMPLATE_VERSION = "c7-rollup-v1";

export const REPORT_TEMPLATES: Readonly<Record<ReportPeriodKind, readonly string[]>> = {
  daily: ["What changed", "Pivots and why", "Permanent remediations", "Activity"],
  weekly: ["Handled last week", "Biggest changes", "Planned next week", "Forward-day sketches", "Classified activity"],
  monthly: ["Roadmap and features", "Struggles and process improvements", "Cost and capability changes", "Surprises and learnings", "Ideas and designs"],
  quarterly: ["Major items", "Roadmap and features", "Cost and traffic patterns", "Surprises and learnings"],
  annual: ["Plans and SaaS", "Dollar and PR efficiency", "Major changes and mistakes", "Surprises", "Next-year ideas"],
};

export interface ClassifiedActivity {
  implemented: string[];
  cancelled: string[];
  duplicate: string[];
  inProgress: string[];
  blocked: string[];
  queued: string[];
}

export interface ComposeReportOptions {
  asOf: string;
  generatedAt?: string;
  /** Cursor is derived from selected immutable input IDs when omitted. */
  sourceCursor?: string;
}

export interface ComposedReport {
  period: ReportPeriod;
  asOf: string;
  templateVersion: string;
  sourceCursor: string;
  sources: ReportRecordSource[];
  blocks: ReportRecordBlock[];
  classifiedActivity: ClassifiedActivity;
}

const classificationKeys = {
  implemented: "implemented",
  cancelled: "cancelled",
  duplicate: "duplicate",
  "in-progress": "inProgress",
  blocked: "blocked",
  queued: "queued",
} as const;

function inPeriod(record: ReportRecord, period: ReportPeriod): boolean {
  const start = Date.parse(record.period.periodStart);
  const end = Date.parse(record.period.periodEnd);
  return start < Date.parse(period.periodEnd) && end > Date.parse(period.periodStart);
}

function preferComposedChildren(records: ReportRecord[], kind: ReportPeriodKind): ReportRecord[] {
  const matching = records.filter((record) => record.period.kind === kind);
  const byPeriod = new Map<string, ReportRecord[]>();
  for (const record of matching) byPeriod.set(record.period.id, [...(byPeriod.get(record.period.id) ?? []), record]);
  return [...byPeriod.values()].flatMap((periodRecords) => {
    const composed = periodRecords.filter((record) => record.templateVersion === REPORT_TEMPLATE_VERSION);
    return composed.length ? composed : periodRecords;
  });
}

function lowerGrainRecords(period: ReportPeriod, records: ReportRecord[]): { record: ReportRecord; reachBack: boolean }[] {
  const daily = preferComposedChildren(records, "daily");
  if (period.kind === "daily") return daily
    .filter((record) => record.templateVersion !== REPORT_TEMPLATE_VERSION && inPeriod(record, period))
    .map((record) => ({ record, reachBack: false }));

  const current = daily.filter((record) => inPeriod(record, period)).map((record) => ({ record, reachBack: false }));
  // Weekly reports carry the prior week's Friday as explicit context. It is
  // kept distinct from the current-week source set and never changes identity.
  if (period.kind === "weekly") {
    const monday = /^weekly:(\d{4}-\d{2}-\d{2}):America\/Chicago$/.exec(period.id)?.[1];
    if (!monday) throw new Error("weekly period has no canonical Monday identity");
    const mondayDate = new Date(`${monday}T00:00:00.000Z`);
    mondayDate.setUTCDate(mondayDate.getUTCDate() - 3);
    const friday = mondayDate.toISOString().slice(0, 10);
    const fridayStart = Date.parse(dailyReportPeriod(friday).periodStart);
    const mondayStart = Date.parse(period.periodStart);
    const reachBack = daily
      .filter((record) => Date.parse(record.period.periodStart) >= fridayStart && Date.parse(record.period.periodStart) < mondayStart)
      .map((record) => ({ record, reachBack: true }));
    return [...current, ...reachBack];
  }

  const grainOrder: ReportPeriodKind[] = ["daily", "weekly", "monthly", "quarterly", "annual"];
  const parentIndex = grainOrder.indexOf(period.kind);
  const childKind = grainOrder[parentIndex - 1];
  return preferComposedChildren(records, childKind!)
    .filter((record) => inPeriod(record, period))
    .map((record) => ({ record, reachBack: false }));
}

function stableSourceRows(period: ReportPeriod, records: ReportRecord[]) {
  return lowerGrainRecords(period, records).sort((left, right) =>
    left.record.period.periodStart.localeCompare(right.record.period.periodStart) || left.record.id.localeCompare(right.record.id),
  );
}

/**
 * Compose one explicit period from durable lower-grain records. Source-time
 * ordering and IDs, rather than arrival order, are the determinism oracle.
 */
export function composeReport(period: ReportPeriod, records: ReportRecord[], options: ComposeReportOptions): ComposedReport {
  if (!Number.isFinite(Date.parse(options.asOf)) || new Date(options.asOf).toISOString() !== options.asOf) {
    throw new Error("asOf must be a canonical ISO-8601 timestamp");
  }
  if (Date.parse(options.asOf) < Date.parse(period.periodStart) || Date.parse(options.asOf) >= Date.parse(period.periodEnd)) {
    throw new Error("asOf must fall within the report period");
  }

  const rows = stableSourceRows(period, records);
  const orderedBlocks = rows.flatMap(({ record, reachBack }) => record.blocks
    .filter((block) => !block.importance || block.importance.includes(period.kind))
    .map((block) => ({
      ...structuredClone(block),
      id: `${record.id}:${block.id}`,
      text: reachBack ? `[Prior-Friday reach-back] ${block.text}` : block.text,
      links: [...new Set([...(block.links ?? []), ...record.sources.map((source) => source.uri)])].sort(),
      importance: block.importance ?? defaultImportance(record.period.kind),
    })))
    .sort((left, right) => left.relatedAt.localeCompare(right.relatedAt) || left.id.localeCompare(right.id));

  const activity: ClassifiedActivity = {
    implemented: [], cancelled: [], duplicate: [], inProgress: [], blocked: [], queued: [],
  };
  for (const block of orderedBlocks) {
    if (block.classification) activity[classificationKeys[block.classification]].push(...(block.links ?? []));
  }
  for (const key of Object.keys(activity) as (keyof ClassifiedActivity)[]) activity[key] = [...new Set(activity[key])].sort();

  const summaryBlocks: ReportRecordBlock[] = (Object.keys(activity) as (keyof ClassifiedActivity)[])
    .filter((key) => activity[key].length > 0)
    .map((key) => ({
      id: `activity:${key}`,
      text: `${key}: ${activity[key].length} linked member${activity[key].length === 1 ? "" : "s"}`,
      relatedAt: options.asOf,
      links: activity[key],
      importance: defaultImportance(period.kind),
    }));
  const sources = [...new Map(rows.flatMap(({ record }) => record.sources).map((source) => [source.id, source])).values()]
    .sort((left, right) => left.observedAt.localeCompare(right.observedAt) || left.id.localeCompare(right.id));
  const sourceCursor = options.sourceCursor ?? rows.map(({ record }) => record.id).sort().join(",");
  if (!sourceCursor.trim()) throw new Error("cannot compose a report without canonical source records");
  const templateBlocks: ReportRecordBlock[] = REPORT_TEMPLATES[period.kind].map((title, index) => ({
    id: `template:section:${String(index + 1).padStart(2, "0")}`,
    text: title,
    relatedAt: period.periodStart,
    importance: defaultImportance(period.kind),
  }));

  return {
    period,
    asOf: options.asOf,
    templateVersion: REPORT_TEMPLATE_VERSION,
    sourceCursor,
    sources,
    blocks: [...templateBlocks, ...summaryBlocks, ...orderedBlocks],
    classifiedActivity: activity,
  };
}

/** Read durable inputs, compose one explicit period, then append idempotently. */
export async function generateReportFromStoredRecords(
  location: ReportRecordLocation,
  period: ReportPeriod,
  options: ComposeReportOptions,
): Promise<{ record: ReportRecord; inserted: boolean }> {
  const stored = await readCurrentReportRecords(location);
  const composed = composeReport(period, stored, options);
  const record = makeReportRecord(composed, options.generatedAt);
  const result = await appendReportRecord(location, record);
  return { record: result.record, inserted: result.inserted };
}

/** Worked fixture from the C7 amendment; progress is append-only prose. */
export const MODAL_LIVING_ITEM_FIXTURE = [
  "Began a 2-day re-embed project using Modal; embedding with 2 models; space +~80% per initial benchmarks; will spend $40 on Modal; must purchase $10 of Modal before 18 hours pass.",
  "Purchased $10; $28 used, 15% to go; projected $6 of purchased quota used.",
] as const;

/** Mark a daily block for the normal upward bubbling path. */
export function defaultImportance(grain: ReportPeriodKind): ReportPeriodKind[] {
  const order: ReportPeriodKind[] = ["daily", "weekly", "monthly", "quarterly", "annual"];
  const index = order.indexOf(grain);
  return order.slice(index);
}
