// Canonical C7 report records stored in the aggregator's append-only PM-stack
// event log. The log is the durable source; renderers consume the latest
// generation for a period/template and retain the source cursor that produced it.
import { createHash } from "node:crypto";
import { appendFile, mkdir, readFile } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { dailyReportPeriod, type DailyReportPeriod } from "./report-periods.ts";

export interface ReportRecordSource {
  id: string;
  uri: string;
  observedAt: string;
}

export interface ReportRecordBlock {
  id: string;
  text: string;
  relatedAt: string;
  links?: string[];
}

export interface ReportRecordInput {
  period: DailyReportPeriod;
  asOf: string;
  templateVersion: string;
  sourceCursor: string;
  sources: ReportRecordSource[];
  blocks: ReportRecordBlock[];
}

export interface ReportRecord extends ReportRecordInput {
  id: string;
  generatedAt: string;
}

/** Persistent PM-stack location; override for tests and managed deployments. */
export function defaultReportRecordsFile(env: Record<string, string | undefined> = process.env): string {
  const configured = env.PM_AGG_REPORT_RECORDS_FILE?.trim();
  return configured || join(homedir(), ".local", "state", "pm-aggregator", "report-records.jsonl");
}

const REPORT_UPDATE_OPERATIONS = ["edit", "replace", "quote", "append", "banner", "event-log", "feedback"] as const;
export type ReportUpdateOperation = (typeof REPORT_UPDATE_OPERATIONS)[number];

const ALLOWED_AFTER_SOFT_CLOSE: ReadonlySet<ReportUpdateOperation> = new Set([
  "quote",
  "append",
  "banner",
  "event-log",
  "feedback",
]);

/** Enforce the report's soft-close allowlist; the close instant is exclusive. */
export function assertReportUpdateAllowed(
  period: DailyReportPeriod,
  operation: ReportUpdateOperation,
  at: string,
): void {
  requireIsoDate(at, "update time");
  if (!(REPORT_UPDATE_OPERATIONS as readonly string[]).includes(operation)) throw new Error(`unknown report operation: ${operation}`);
  if (Date.parse(at) < Date.parse(period.softCloseAt)) return;
  if (!ALLOWED_AFTER_SOFT_CLOSE.has(operation)) {
    throw new Error(`report operation ${operation} is not allowed after soft close ${period.softCloseAt}`);
  }
}

export interface ReportSnapshot {
  id: string;
  reportRecordId: string;
  snapshotAt: string;
  record: ReportRecord;
}

type ReportRecordEvent = {
  eventId: string;
  type: "report.generated" | "report.snapshot";
  record?: ReportRecord;
  snapshot?: ReportSnapshot;
  ts: string;
};

export class ReportRecordConflictError extends Error {
  constructor(recordId: string) {
    super(`report record ${recordId} already exists with different content`);
    this.name = "ReportRecordConflictError";
  }
}

/** Stable across reruns for the same period, template and source cursor. */
export function reportRecordId(input: Pick<ReportRecordInput, "period" | "templateVersion" | "sourceCursor">): string {
  const key = `${input.period.id}\n${input.templateVersion}\n${input.sourceCursor}`;
  return `report:${createHash("sha256").update(key).digest("hex")}`;
}

/** Build a validated canonical record; generatedAt is injectable for replay. */
export function makeReportRecord(input: ReportRecordInput, generatedAt = new Date().toISOString()): ReportRecord {
  requireIsoDate(input.asOf, "asOf");
  requireIsoDate(generatedAt, "generatedAt");
  if (!input.templateVersion.trim()) throw new Error("templateVersion is required");
  if (!input.sourceCursor.trim()) throw new Error("sourceCursor is required");
  if (input.period.timezone !== "America/Chicago") throw new Error("report period must use America/Chicago");
  const date = /^daily:(\d{4}-\d{2}-\d{2}):America\/Chicago$/.exec(input.period.id)?.[1];
  const canonicalPeriod = date ? dailyReportPeriod(date) : null;
  if (
    !canonicalPeriod ||
    input.period.kind !== canonicalPeriod.kind ||
    input.period.periodStart !== canonicalPeriod.periodStart ||
    input.period.periodEnd !== canonicalPeriod.periodEnd ||
    input.period.softCloseAt !== canonicalPeriod.softCloseAt
  ) {
    throw new Error("period must be a canonical dailyReportPeriod value");
  }
  if (Date.parse(input.asOf) < Date.parse(input.period.periodStart) || Date.parse(input.asOf) >= Date.parse(input.period.periodEnd)) {
    throw new Error("asOf must fall within the report period");
  }
  if (!Array.isArray(input.sources) || !Array.isArray(input.blocks)) throw new Error("sources and blocks must be arrays");
  for (const source of input.sources) {
    if (!source.id.trim() || !source.uri.trim()) throw new Error("each source requires id and uri");
    requireIsoDate(source.observedAt, "source observedAt");
  }
  const blockIds = new Set<string>();
  for (const block of input.blocks) {
    if (!block.id.trim() || !block.text.trim()) throw new Error("each report block requires id and text");
    if (blockIds.has(block.id)) throw new Error(`duplicate report block id: ${block.id}`);
    blockIds.add(block.id);
    requireIsoDate(block.relatedAt, "block relatedAt");
  }
  return { ...structuredClone(input), id: reportRecordId(input), generatedAt };
}

function requireIsoDate(value: string, name: string): void {
  if (!Number.isFinite(Date.parse(value)) || new Date(value).toISOString() !== value) {
    throw new Error(`${name} must be a canonical ISO-8601 timestamp`);
  }
}

function parseEvent(line: string): ReportRecordEvent | null {
  try {
    const value: unknown = JSON.parse(line);
    if (!value || typeof value !== "object" || Array.isArray(value)) return null;
    const event = value as Record<string, unknown>;
    if (typeof event.eventId !== "string" || typeof event.ts !== "string") return null;
    if (event.type === "report.generated" && event.record && typeof event.record === "object" && !Array.isArray(event.record)) return value as ReportRecordEvent;
    if (event.type === "report.snapshot" && event.snapshot && typeof event.snapshot === "object" && !Array.isArray(event.snapshot)) return value as ReportRecordEvent;
    return null;
  } catch {
    return null; // Ignore a malformed/torn final line, matching decision-events.ts.
  }
}

async function readEvents(file: string): Promise<ReportRecordEvent[]> {
  let text: string;
  try {
    text = await readFile(file, "utf8");
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return [];
    throw error;
  }
  return text.split("\n").filter((line) => line.trim()).map(parseEvent).filter((event): event is ReportRecordEvent => event !== null);
}

/** Keep a torn final JSONL fragment from swallowing the next acknowledged event. */
async function terminateTornTail(file: string): Promise<void> {
  let text: string;
  try {
    text = await readFile(file, "utf8");
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return;
    throw error;
  }
  if (text.length > 0 && !text.endsWith("\n")) await appendFile(file, "\n", "utf8");
}

// Serialize writes in this process, as decision-events.ts does. A future
// multi-process adapter can replace this seam without changing record callers.
const appendChains = new Map<string, Promise<unknown>>();
function serializeAppend<T>(file: string, fn: () => Promise<T>): Promise<T> {
  const previous = appendChains.get(file) ?? Promise.resolve();
  const run = previous.then(fn, fn);
  appendChains.set(file, run.catch(() => {}));
  return run;
}

export interface AppendReportRecordResult {
  inserted: boolean;
  record: ReportRecord;
}

/**
 * Append once by stable record ID. A byte-equivalent rerun is a no-op; the
 * same id with changed content is a conflict and is never silently replaced.
 */
export async function appendReportRecord(file: string, record: ReportRecord): Promise<AppendReportRecordResult> {
  assertCanonicalRecord(record);
  return serializeAppend(file, async () => {
    const events = await readEvents(file);
    const existing = events.find((event) => event.type === "report.generated" && event.record?.id === record.id)?.record;
    if (existing) {
      if (!sameRecordContent(existing, record)) throw new ReportRecordConflictError(record.id);
      return { inserted: false, record: existing };
    }
    await mkdir(dirname(file), { recursive: true });
    await terminateTornTail(file);
    const event: ReportRecordEvent = {
      eventId: record.id,
      type: "report.generated",
      record,
      ts: record.generatedAt,
    };
    await appendFile(file, `${JSON.stringify(event)}\n`, "utf8");
    return { inserted: true, record };
  });
}

/** Read the latest record per stable ID in append order (currently one per key). */
export async function readReportRecords(file: string): Promise<ReportRecord[]> {
  const byId = new Map<string, ReportRecord>();
  for (const event of await readEvents(file)) {
    if (event.type === "report.generated" && event.record) byId.set(event.record.id, event.record);
  }
  return [...byId.values()];
}

/** Current projection: latest appended source-cursor version per period/template. */
export async function readCurrentReportRecords(file: string): Promise<ReportRecord[]> {
  const current = new Map<string, ReportRecord>();
  for (const event of await readEvents(file)) {
    if (event.type !== "report.generated" || !event.record) continue;
    current.set(`${event.record.period.id}\n${event.record.templateVersion}`, event.record);
  }
  return [...current.values()];
}

export interface AppendReportSnapshotResult {
  inserted: boolean;
  snapshot: ReportSnapshot;
}

/** Freeze a dated view in the same append-only PM-stack log. Existing snapshot
 * IDs are immutable: a repeated identical write is idempotent, changed content
 * is a conflict that must become a separately linked correction record. */
export async function appendReportSnapshot(
  file: string,
  record: ReportRecord,
  snapshotAt: string,
): Promise<AppendReportSnapshotResult> {
  assertCanonicalRecord(record);
  requireIsoDate(snapshotAt, "snapshotAt");
  const snapshot: ReportSnapshot = {
    id: `snapshot:${record.period.id}`,
    reportRecordId: record.id,
    snapshotAt,
    record: structuredClone(record),
  };
  return serializeAppend(file, async () => {
    const events = await readEvents(file);
    const generated = events.find((event) => event.type === "report.generated" && event.record?.id === record.id)?.record;
    if (generated && !sameRecordContent(generated, record)) throw new ReportRecordConflictError(record.id);
    const existing = events.find((event) => event.type === "report.snapshot" && event.snapshot?.id === snapshot.id)?.snapshot;
    if (existing) {
      if (existing.reportRecordId !== snapshot.reportRecordId || !sameRecordContent(existing.record, snapshot.record)) {
        throw new ReportRecordConflictError(snapshot.id);
      }
      return { inserted: false, snapshot: existing };
    }
    await mkdir(dirname(file), { recursive: true });
    await terminateTornTail(file);
    const event: ReportRecordEvent = {
      eventId: snapshot.id,
      type: "report.snapshot",
      snapshot,
      ts: snapshotAt,
    };
    await appendFile(file, `${JSON.stringify(event)}\n`, "utf8");
    return { inserted: true, snapshot };
  });
}

export async function readReportSnapshots(file: string): Promise<ReportSnapshot[]> {
  return (await readEvents(file))
    .filter((event): event is ReportRecordEvent & { snapshot: ReportSnapshot } => event.type === "report.snapshot" && !!event.snapshot)
    .map((event) => event.snapshot);
}

function sameRecordContent(left: ReportRecord, right: ReportRecord): boolean {
  const { generatedAt: _leftTime, ...leftContent } = left;
  const { generatedAt: _rightTime, ...rightContent } = right;
  return canonicalJson(leftContent) === canonicalJson(rightContent);
}

function assertCanonicalRecord(record: ReportRecord): void {
  const canonical = makeReportRecord(record, record.generatedAt);
  if (!sameRecordContent(canonical, record)) {
    throw new Error("report record does not match its canonical contents and ID");
  }
}

function canonicalJson(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  if (typeof value === "object" && value !== null) {
    const object = value as Record<string, unknown>;
    return `{${Object.keys(object).sort().map((key) => `${JSON.stringify(key)}:${canonicalJson(object[key])}`).join(",")}}`;
  }
  return JSON.stringify(value) ?? "null";
}

/** Whitespace-only edits are equivalent; any substantive divergence is a conflict. */
export function reportBlockConflict(storedText: string, renderedText: string): boolean {
  const normalize = (text: string) => text.replace(/\s+/g, " ").trim();
  return normalize(storedText) !== normalize(renderedText);
}
