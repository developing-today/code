// Deterministic C7 period rollups over canonical, persisted lower-grain records.
import { dailyReportPeriod, type ReportPeriod, type ReportPeriodKind } from "./report-periods.ts";
import type { ReportRecord, ReportRecordBlock, ReportRecordSource, ReportRecordLocation } from "./report-records.ts";
import { appendReportRecord, makeReportRecord, readReportRecords } from "./report-records.ts";

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
  bubblingCandidates: ReportRecordBlock[];
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
  const daily = period.kind === "daily"
    ? records.filter((record) => record.period.kind === "daily" && record.templateVersion !== REPORT_TEMPLATE_VERSION)
    : preferComposedChildren(records, "daily");
  if (period.kind === "daily") return daily
    .filter((record) => inPeriod(record, period))
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
    const fridayPeriod = dailyReportPeriod(friday);
    const reachBack = daily
      .filter((record) => record.period.id === fridayPeriod.id)
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

  const rows = stableSourceRows(period, records.filter((record) => Date.parse(record.asOf) <= Date.parse(options.asOf)));
  if (!rows.length) throw new Error("cannot compose a report without canonical source records");
  const orderedBlocks: ReportRecordBlock[] = [];
  const bubblingCandidates: ReportRecordBlock[] = [];
  const orderedByIdentity = new Map<string, { block: ReportRecordBlock; reachBack: boolean }>();
  const grainOrder: ReportPeriodKind[] = ["daily", "weekly", "monthly", "quarterly", "annual"];
  const targetGrain = grainOrder.indexOf(period.kind);
  for (const { record, reachBack } of rows) {
    const composedInput = record.templateVersion === REPORT_TEMPLATE_VERSION;
    const sourceBlocks = [...new Map(
      [...record.blocks, ...(record.bubblingCandidates ?? [])].map((block) => [block.id, block]),
    ).values()];
    for (const block of sourceBlocks) {
      // Templates and summaries describe their source report, not source
      // activity. Rebuild them at the target grain instead of rolling them up.
      if (composedInput && (block.id.startsWith("template:") || block.id.startsWith("activity:"))) continue;
      const relatedAt = Date.parse(block.relatedAt);
      const inRequestedPeriod = relatedAt >= Date.parse(period.periodStart) && relatedAt < Date.parse(period.periodEnd);
      if (relatedAt > Date.parse(options.asOf) || (!inRequestedPeriod && !reachBack)) continue;
      const importance = block.importance ?? defaultImportance(record.period.kind);
      const normalized = { ...structuredClone(block), importance };
      const id = composedInput ? block.id : `${record.id}:${block.id}`;
      if (importance.includes(period.kind)) {
        const ordered = {
          ...normalized,
          id,
          text: reachBack ? `[Prior-Friday reach-back] ${block.text}` : block.text,
          links: block.links ? [...new Set(block.links)].sort() : undefined,
        };
        const previous = orderedByIdentity.get(id);
        if (!previous || (previous.reachBack && !reachBack)) orderedByIdentity.set(id, { block: ordered, reachBack });
      }
      if (!reachBack && importance.some((kind) => grainOrder.indexOf(kind) > targetGrain)) {
        bubblingCandidates.push({ ...normalized, id });
      }
    }
  }
  orderedBlocks.push(...[...orderedByIdentity.values()].map(({ block }) => block));
  const uniqueCandidates = [...new Map(bubblingCandidates.map((block) => [block.id, block])).values()];
  orderedBlocks.sort((left, right) => left.relatedAt.localeCompare(right.relatedAt) || left.id.localeCompare(right.id));

  const activity: ClassifiedActivity = {
    implemented: [], cancelled: [], duplicate: [], inProgress: [], blocked: [], queued: [],
  };
  const latestClassification = new Map<string, { relatedAt: string; id: string; key: keyof ClassifiedActivity }>();
  for (const block of orderedBlocks) {
    if (!block.classification) continue;
    const key = classificationKeys[block.classification];
    for (const member of block.links ?? []) {
      const previous = latestClassification.get(member);
      if (!previous || block.relatedAt.localeCompare(previous.relatedAt) > 0 ||
        (block.relatedAt === previous.relatedAt && block.id.localeCompare(previous.id) > 0)) {
        latestClassification.set(member, { relatedAt: block.relatedAt, id: block.id, key });
      }
    }
  }
  for (const [member, latest] of latestClassification) activity[latest.key].push(member);
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
  const sources = [...new Map(rows.flatMap(({ record }) => record.sources)
    .filter((source) => Date.parse(source.observedAt) <= Date.parse(options.asOf))
    .map((source) => [source.id, source])).values()]
    .sort((left, right) => left.observedAt.localeCompare(right.observedAt) || left.id.localeCompare(right.id));
  const sourceCursor = options.sourceCursor ?? `${rows.map(({ record }) => record.id).sort().join(",")};asOf:${options.asOf}`;
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
    bubblingCandidates: uniqueCandidates,
    classifiedActivity: activity,
  };
}

/** Read durable inputs, compose one explicit period, then append idempotently. */
export async function generateReportFromStoredRecords(
  location: ReportRecordLocation,
  period: ReportPeriod,
  options: ComposeReportOptions,
): Promise<{ record: ReportRecord; inserted: boolean }> {
  const allStored = await readReportRecords(location);
  const latestAsOf = new Map<string, ReportRecord>();
  for (const record of allStored) {
    if (Date.parse(record.asOf) > Date.parse(options.asOf)) continue;
    const key = `${record.period.id}\n${record.templateVersion}`;
    const previous = latestAsOf.get(key);
    if (!previous || record.asOf > previous.asOf || (record.asOf === previous.asOf && record.generatedAt > previous.generatedAt)) {
      latestAsOf.set(key, record);
    }
  }
  const stored = [...latestAsOf.values()];
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
