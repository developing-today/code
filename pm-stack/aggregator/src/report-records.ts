// Canonical C7 report records over PM-stack append-only storage. Local JSONL
// supports development/rollback; the durable adapter appends immutable event
// documents to the existing aggregator document store.
import { createHash, randomUUID } from "node:crypto";
import { appendFile, link, mkdir, readFile, unlink, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import type { AppendOnlyDocumentRemote } from "./aa-last-good.ts";
import { reportPeriod, type ReportPeriod, type ReportPeriodKind } from "./report-periods.ts";

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
  importance?: ReportPeriodKind[];
  classification?: "implemented" | "cancelled" | "duplicate" | "in-progress" | "blocked" | "queued";
  updateOperation?: ReportLateUpdateOperation;
  resolution?: {
    sourceRecordId: string;
    comparedRecordId?: string;
    strategy: "stored" | "rendered" | "edited" | "combined";
    resolvedAt: string;
  };
}

export interface ReportRecordInput {
  period: ReportPeriod;
  asOf: string;
  templateVersion: string;
  sourceCursor: string;
  sources: ReportRecordSource[];
  blocks: ReportRecordBlock[];
  /** Non-visible blocks retained for explicitly requested higher-grain rollups. */
  bubblingCandidates?: ReportRecordBlock[];
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
export type ReportLateUpdateOperation = Extract<
  ReportUpdateOperation,
  "quote" | "append" | "banner" | "event-log" | "feedback"
>;
const REPORT_PERIOD_KINDS = ["daily", "weekly", "monthly", "quarterly", "annual"] as const;
const REPORT_CLASSIFICATIONS = ["implemented", "cancelled", "duplicate", "in-progress", "blocked", "queued"] as const;

const ALLOWED_AFTER_SOFT_CLOSE: ReadonlySet<ReportUpdateOperation> = new Set([
  "quote",
  "append",
  "banner",
  "event-log",
  "feedback",
]);

/** Enforce the report's soft-close allowlist; the close instant is exclusive. */
export function assertReportUpdateAllowed(period: ReportPeriod, operation: ReportUpdateOperation, at: string): void {
  requireIsoDate(at, "update time");
  if (!(REPORT_UPDATE_OPERATIONS as readonly string[]).includes(operation))
    throw new Error(`unknown report operation: ${operation}`);
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

export type ReportRecordEvent = {
  eventId: string;
  type: "report.generated" | "report.snapshot" | "report.snapshot-correction";
  record?: ReportRecord;
  snapshot?: ReportSnapshot;
  correction?: ReportSnapshotCorrection;
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
  const key = JSON.stringify([input.period.id, input.templateVersion, input.sourceCursor]);
  return `report:${createHash("sha256").update(key).digest("hex")}`;
}

/** Build a validated canonical record; generatedAt is injectable for replay. */
export function makeReportRecord(input: ReportRecordInput, generatedAt = new Date().toISOString()): ReportRecord {
  requireIsoDate(input.asOf, "asOf");
  requireIsoDate(generatedAt, "generatedAt");
  if (!input.templateVersion.trim()) throw new Error("templateVersion is required");
  if (!input.sourceCursor.trim()) throw new Error("sourceCursor is required");
  if (input.period.timezone !== "America/Chicago") throw new Error("report period must use America/Chicago");
  const match = /^(daily|weekly|monthly|quarterly|annual):(.+):America\/Chicago$/.exec(input.period.id);
  const kind = match?.[1] as ReportPeriodKind | undefined;
  const identity = match?.[2];
  let canonicalPeriod: ReportPeriod | null = null;
  if (kind && identity) {
    try {
      canonicalPeriod = reportPeriod(kind, identity);
    } catch {
      /* common validation error below */
    }
  }
  if (
    !canonicalPeriod ||
    input.period.kind !== canonicalPeriod.kind ||
    input.period.periodStart !== canonicalPeriod.periodStart ||
    input.period.periodEnd !== canonicalPeriod.periodEnd ||
    input.period.softCloseAt !== canonicalPeriod.softCloseAt
  ) {
    throw new Error("period must be a canonical C7 reportPeriod value");
  }
  if (
    Date.parse(input.asOf) < Date.parse(input.period.periodStart) ||
    Date.parse(input.asOf) >= Date.parse(input.period.periodEnd)
  ) {
    throw new Error("asOf must fall within the report period");
  }
  if (!Array.isArray(input.sources) || !Array.isArray(input.blocks))
    throw new Error("sources and blocks must be arrays");
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
    if (
      block.importance &&
      (!Array.isArray(block.importance) ||
        block.importance.some((kind) => !(REPORT_PERIOD_KINDS as readonly string[]).includes(kind)) ||
        new Set(block.importance).size !== block.importance.length)
    ) {
      throw new Error(`report block ${block.id} has invalid importance levels`);
    }
    if (block.classification && !(REPORT_CLASSIFICATIONS as readonly string[]).includes(block.classification)) {
      throw new Error(`report block ${block.id} has an invalid activity classification`);
    }
    if (block.classification && (!block.links || block.links.length === 0)) {
      throw new Error(`classified activity block ${block.id} must link its members`);
    }
    if (block.updateOperation && !ALLOWED_AFTER_SOFT_CLOSE.has(block.updateOperation)) {
      throw new Error(`report block ${block.id} has an invalid late update operation`);
    }
    if (block.resolution) {
      if (
        !block.resolution.sourceRecordId.trim() ||
        !["stored", "rendered", "edited", "combined"].includes(block.resolution.strategy)
      ) {
        throw new Error(`report block ${block.id} has invalid conflict resolution metadata`);
      }
      requireIsoDate(block.resolution.resolvedAt, "block resolution resolvedAt");
    }
    if (
      block.links &&
      (!Array.isArray(block.links) || block.links.some((link) => typeof link !== "string" || !link.trim()))
    ) {
      throw new Error(`report block ${block.id} has invalid links`);
    }
  }
  for (const block of input.bubblingCandidates ?? []) {
    if (!block.id.trim() || !block.text.trim()) throw new Error("each bubbling candidate requires an id and text");
    requireIsoDate(block.relatedAt, "bubbling candidate relatedAt");
    if (
      block.importance &&
      (!Array.isArray(block.importance) ||
        block.importance.some((kind) => !(REPORT_PERIOD_KINDS as readonly string[]).includes(kind)) ||
        new Set(block.importance).size !== block.importance.length)
    ) {
      throw new Error(`bubbling candidate ${block.id} has invalid importance levels`);
    }
    if (
      block.links &&
      (!Array.isArray(block.links) || block.links.some((link) => typeof link !== "string" || !link.trim()))
    ) {
      throw new Error(`bubbling candidate ${block.id} has invalid links`);
    }
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
    if (
      event.type === "report.generated" &&
      event.record &&
      typeof event.record === "object" &&
      !Array.isArray(event.record)
    )
      return value as ReportRecordEvent;
    if (
      event.type === "report.snapshot" &&
      event.snapshot &&
      typeof event.snapshot === "object" &&
      !Array.isArray(event.snapshot)
    )
      return value as ReportRecordEvent;
    if (
      event.type === "report.snapshot-correction" &&
      event.correction &&
      typeof event.correction === "object" &&
      !Array.isArray(event.correction)
    )
      return value as ReportRecordEvent;
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
  return text
    .split("\n")
    .filter((line) => line.trim())
    .map(parseEvent)
    .filter((event): event is ReportRecordEvent => event !== null);
}

export type ReportRecordLocation = string | AppendOnlyDocumentRemote;
const REPORT_DOCUMENT_PREFIX = "c7-report:v1:";
const REPORT_RECORD_CLAIM_PREFIX = "c7-report-record:v1:";
const REPORT_RESOLUTION_CLAIM_PREFIX = "c7-report-resolution-source:v1:";
const REPORT_CURRENT_CLAIM_PREFIX = "c7-report-current:v1:";

function reportDocumentKey(event: ReportRecordEvent): string {
  return `${REPORT_DOCUMENT_PREFIX}${event.type}:${createHash("sha256").update(event.eventId).digest("hex")}`;
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function parseDocumentEvent(value: unknown): ReportRecordEvent {
  if (!isObject(value) || typeof value.eventId !== "string" || typeof value.ts !== "string") {
    throw new Error("invalid C7 report event in permanent document store");
  }
  if (value.type === "report.generated" && isObject(value.record)) return value as ReportRecordEvent;
  if (value.type === "report.snapshot" && isObject(value.snapshot)) return value as ReportRecordEvent;
  if (value.type === "report.snapshot-correction" && isObject(value.correction)) return value as ReportRecordEvent;
  throw new Error("unknown C7 report event in permanent document store");
}

async function readLocationEvents(location: ReportRecordLocation): Promise<ReportRecordEvent[]> {
  if (typeof location === "string") return readEvents(location);
  const rows = await location.listPrefix(REPORT_DOCUMENT_PREFIX);
  const events = rows.map(({ store, document }) => {
    if (!store.startsWith(REPORT_DOCUMENT_PREFIX))
      throw new Error("permanent report store returned a key outside its namespace");
    return parseDocumentEvent(document);
  });
  // The table's stable key ordering is arbitrary; projections need event-time order.
  return events.sort((left, right) => left.ts.localeCompare(right.ts) || left.eventId.localeCompare(right.eventId));
}

function sameReportEvent(left: ReportRecordEvent, right: ReportRecordEvent): boolean {
  if (left.type !== right.type || left.eventId !== right.eventId) return false;
  if (left.type === "report.generated" && right.type === "report.generated") {
    return !!left.record && !!right.record && sameRecordContent(left.record, right.record);
  }
  if (left.type === "report.snapshot" && right.type === "report.snapshot") {
    return (
      !!left.snapshot &&
      !!right.snapshot &&
      left.snapshot.id === right.snapshot.id &&
      left.snapshot.reportRecordId === right.snapshot.reportRecordId &&
      sameRecordContent(left.snapshot.record, right.snapshot.record)
    );
  }
  if (left.type === "report.snapshot-correction" && right.type === "report.snapshot-correction") {
    return (
      !!left.correction &&
      !!right.correction &&
      left.correction.id === right.correction.id &&
      left.correction.snapshotId === right.correction.snapshotId &&
      left.correction.originalRecordId === right.correction.originalRecordId &&
      sameRecordContent(left.correction.record, right.correction.record)
    );
  }
  return false;
}

async function claimRemoteRecord(location: AppendOnlyDocumentRemote, record: ReportRecord): Promise<void> {
  const key = `${REPORT_RECORD_CLAIM_PREFIX}${createHash("sha256").update(record.id).digest("hex")}`;
  if (await location.insertIfAbsent(key, 1, record)) return;
  const stored = await location.load(key);
  if (!stored || !isObject(stored.document) || typeof stored.document.id !== "string") {
    throw new Error(`permanent report identity claim disappeared or is malformed: ${record.id}`);
  }
  if (!sameRecordContent(stored.document as unknown as ReportRecord, record))
    throw new ReportRecordConflictError(record.id);
}

/** A source record may have only one winning resolution append. The durable
 * claim is shared across processes; same-resolution retries may finish an
 * interrupted append, while a different choice from the same source is stale. */
async function claimResolutionSource(
  location: ReportRecordLocation,
  sourceRecordId: string,
  resolvedRecord: ReportRecord,
): Promise<ReportRecord> {
  const claim = { sourceRecordId, resolvedRecordId: resolvedRecord.id, resolvedRecord };
  const key = `${REPORT_RESOLUTION_CLAIM_PREFIX}${createHash("sha256").update(sourceRecordId).digest("hex")}`;
  if (typeof location !== "string") {
    if (await location.insertIfAbsent(key, 1, claim)) return resolvedRecord;
    const stored = await location.load(key);
    if (!stored || !isObject(stored.document) || stored.document.sourceRecordId !== sourceRecordId) {
      throw new Error(`permanent report resolution claim is malformed: ${sourceRecordId}`);
    }
    if (stored.document.resolvedRecordId !== resolvedRecord.id) {
      throw new Error("stale report block comparison: another resolution already won");
    }
    if (!isObject(stored.document.resolvedRecord) || stored.document.resolvedRecord.id !== resolvedRecord.id) {
      throw new Error(`permanent report resolution claim is missing its canonical record: ${sourceRecordId}`);
    }
    return stored.document.resolvedRecord as unknown as ReportRecord;
  }

  const claimsFile = `${location}.resolution-claims/${createHash("sha256").update(sourceRecordId).digest("hex")}.json`;
  await mkdir(dirname(claimsFile), { recursive: true });
  const tempFile = `${claimsFile}.${process.pid}.${randomUUID()}.tmp`;
  try {
    await writeFile(tempFile, JSON.stringify(claim), { encoding: "utf8", flag: "wx" });
    await link(tempFile, claimsFile);
    return resolvedRecord;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error;
    let stored: unknown;
    try {
      stored = JSON.parse(await readFile(claimsFile, "utf8")) as unknown;
    } catch {
      throw new Error(`local report resolution claim is malformed: ${sourceRecordId}`);
    }
    if (!isObject(stored) || stored.sourceRecordId !== sourceRecordId) {
      throw new Error(`local report resolution claim is malformed: ${sourceRecordId}`);
    }
    if (stored.resolvedRecordId !== resolvedRecord.id) {
      throw new Error("stale report block comparison: another resolution already won");
    }
    if (!isObject(stored.resolvedRecord) || stored.resolvedRecord.id !== resolvedRecord.id) {
      throw new Error(`local report resolution claim is missing its canonical record: ${sourceRecordId}`);
    }
    return stored.resolvedRecord as unknown as ReportRecord;
  } finally {
    await unlink(tempFile).catch(() => undefined);
  }
}

async function readResolutionSourceClaim(
  location: ReportRecordLocation,
  sourceRecordId: string,
): Promise<ReportRecord | null> {
  const key = `${REPORT_RESOLUTION_CLAIM_PREFIX}${createHash("sha256").update(sourceRecordId).digest("hex")}`;
  if (typeof location !== "string") {
    const stored = await location.load(key);
    if (!stored) return null;
    if (
      !isObject(stored.document) ||
      stored.document.sourceRecordId !== sourceRecordId ||
      !isObject(stored.document.resolvedRecord) ||
      typeof stored.document.resolvedRecord.id !== "string"
    ) {
      throw new Error(`permanent report resolution claim is malformed: ${sourceRecordId}`);
    }
    return stored.document.resolvedRecord as unknown as ReportRecord;
  }

  const claimsFile = `${location}.resolution-claims/${createHash("sha256").update(sourceRecordId).digest("hex")}.json`;
  let stored: unknown;
  try {
    stored = JSON.parse(await readFile(claimsFile, "utf8")) as unknown;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return null;
    throw new Error(`local report resolution claim is malformed: ${sourceRecordId}`);
  }
  if (
    !isObject(stored) ||
    stored.sourceRecordId !== sourceRecordId ||
    !isObject(stored.resolvedRecord) ||
    typeof stored.resolvedRecord.id !== "string"
  ) {
    throw new Error(`local report resolution claim is malformed: ${sourceRecordId}`);
  }
  return stored.resolvedRecord as unknown as ReportRecord;
}

/** Atomically reserve the next current record from the expected current head.
 * Generation and resolution both use this claim, so neither can race the other. */
async function claimCurrentReportTransition(
  location: ReportRecordLocation,
  previous: ReportRecord | undefined,
  candidate: ReportRecord,
): Promise<ReportRecord> {
  const periodId = candidate.period.id;
  const templateVersion = candidate.templateVersion;
  const previousRecordId = previous?.id ?? null;
  const identity = JSON.stringify([periodId, templateVersion, previousRecordId]);
  const digest = createHash("sha256").update(identity).digest("hex");
  const claim = { periodId, templateVersion, previousRecordId, record: candidate };
  const assertWinner = async (stored: unknown): Promise<ReportRecord> => {
    if (
      !isObject(stored) ||
      stored.periodId !== periodId ||
      stored.templateVersion !== templateVersion ||
      stored.previousRecordId !== previousRecordId ||
      !isObject(stored.record) ||
      typeof stored.record.id !== "string"
    ) {
      throw new Error(`report current-head claim is malformed: ${periodId}/${templateVersion}`);
    }
    const winner = stored.record as unknown as ReportRecord;
    assertCanonicalRecord(winner);
    if (winner.period.id !== periodId || winner.templateVersion !== templateVersion) {
      throw new Error(`report current-head claim is malformed: ${periodId}/${templateVersion}`);
    }
    if (winner.id !== candidate.id || !sameRecordContent(winner, candidate)) {
      await recoverClaimedReportRecord(location, winner);
      throw new Error("stale report block comparison: report record is no longer current");
    }
    return winner;
  };

  if (typeof location !== "string") {
    const key = `${REPORT_CURRENT_CLAIM_PREFIX}${digest}`;
    if (await location.insertIfAbsent(key, 1, claim)) return candidate;
    const stored = await location.load(key);
    if (!stored) throw new Error(`report current-head claim disappeared: ${periodId}/${templateVersion}`);
    return assertWinner(stored.document);
  }

  const claimsFile = `${location}.current-claims/${digest}.json`;
  await mkdir(dirname(claimsFile), { recursive: true });
  const tempFile = `${claimsFile}.${process.pid}.${randomUUID()}.tmp`;
  try {
    await writeFile(tempFile, JSON.stringify(claim), { encoding: "utf8", flag: "wx" });
    await link(tempFile, claimsFile);
    return candidate;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error;
    let stored: unknown;
    try {
      stored = JSON.parse(await readFile(claimsFile, "utf8")) as unknown;
    } catch {
      throw new Error(`local report current-head claim is malformed: ${periodId}/${templateVersion}`);
    }
    return assertWinner(stored);
  } finally {
    await unlink(tempFile).catch(() => undefined);
  }
}

async function appendDocumentEvent(
  location: AppendOnlyDocumentRemote,
  event: ReportRecordEvent,
): Promise<{ inserted: boolean; event: ReportRecordEvent }> {
  const key = reportDocumentKey(event);
  if (await location.insertIfAbsent(key, 1, event)) return { inserted: true, event };
  const stored = await location.load(key);
  if (!stored) throw new Error(`permanent report event disappeared after duplicate insert: ${event.eventId}`);
  const existing = parseDocumentEvent(stored.document);
  if (!sameReportEvent(existing, event)) throw new ReportRecordConflictError(event.eventId);
  return { inserted: false, event: existing };
}

/** Finish a durable head winner whose claim was written before its event append failed. */
async function recoverClaimedReportRecord(location: ReportRecordLocation, record: ReportRecord): Promise<void> {
  const event: ReportRecordEvent = {
    eventId: record.id,
    type: "report.generated",
    record,
    ts: record.generatedAt,
  };
  if (typeof location !== "string") {
    await appendDocumentEvent(location, event);
    return;
  }

  const events = await readEvents(location);
  const existing = events.find((item) => item.type === "report.generated" && item.record?.id === record.id)?.record;
  if (existing) {
    if (!sameRecordContent(existing, record)) throw new ReportRecordConflictError(record.id);
    return;
  }
  await mkdir(dirname(location), { recursive: true });
  await terminateTornTail(location);
  await appendFile(location, `${JSON.stringify(event)}\n`, "utf8");
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
  appendChains.set(
    file,
    run.catch(() => {}),
  );
  return run;
}

const remoteAppendChains = new WeakMap<object, Promise<unknown>>();
function serializeRemoteAppend<T>(location: AppendOnlyDocumentRemote, fn: () => Promise<T>): Promise<T> {
  const previous = remoteAppendChains.get(location as object) ?? Promise.resolve();
  const run = previous.then(fn, fn);
  remoteAppendChains.set(
    location as object,
    run.catch(() => {}),
  );
  return run;
}

function serializeLocationAppend<T>(location: ReportRecordLocation, fn: () => Promise<T>): Promise<T> {
  return typeof location === "string" ? serializeAppend(location, fn) : serializeRemoteAppend(location, fn);
}

export interface AppendReportRecordResult {
  inserted: boolean;
  record: ReportRecord;
}

/**
 * Append once by stable record ID. A content-equivalent rerun is a no-op; the
 * same id with changed content is a conflict and is never silently replaced.
 */
async function appendReportRecordUnlocked(
  location: ReportRecordLocation,
  record: ReportRecord,
  expectedPreviousRecordId?: string,
  operation: ReportUpdateOperation = "replace",
): Promise<AppendReportRecordResult> {
  assertCanonicalRecord(record);
  let events: ReportRecordEvent[];
  if (typeof location !== "string") {
    events = await readLocationEvents(location);
    assertRecordIdContent(events, record);
    const existing = events.find((item) => item.type === "report.generated" && item.record?.id === record.id)?.record;
    if (existing) {
      if (!sameRecordContent(existing, record)) throw new ReportRecordConflictError(record.id);
      return { inserted: false, record: existing };
    }
    const previous = currentReportRecord(events, record);
    if (expectedPreviousRecordId !== undefined && previous?.id !== expectedPreviousRecordId) {
      throw new Error("stale report block comparison: report record is no longer current");
    }
    if (previous && Date.parse(record.generatedAt) <= Date.parse(previous.generatedAt)) {
      throw new Error("stale report block comparison: report timestamp does not advance the current record");
    }
    if (previous) assertReportUpdateAllowed(record.period, operation, record.generatedAt);
    await claimRemoteRecord(location, record);
    const claimedRecord = await claimCurrentReportTransition(location, previous, record);
    const event: ReportRecordEvent = {
      eventId: claimedRecord.id,
      type: "report.generated",
      record: claimedRecord,
      ts: claimedRecord.generatedAt,
    };
    const result = await appendDocumentEvent(location, event);
    if (result.event.type !== "report.generated" || !result.event.record)
      throw new Error("permanent report record has an invalid event type");
    return { inserted: result.inserted, record: result.event.record };
  }
  events = await readEvents(location);
  assertRecordIdContent(events, record);
  const existing = events.find((item) => item.type === "report.generated" && item.record?.id === record.id)?.record;
  if (existing) {
    if (!sameRecordContent(existing, record)) throw new ReportRecordConflictError(record.id);
    return { inserted: false, record: existing };
  }
  const previous = currentReportRecord(events, record);
  if (expectedPreviousRecordId !== undefined && previous?.id !== expectedPreviousRecordId) {
    throw new Error("stale report block comparison: report record is no longer current");
  }
  if (previous && Date.parse(record.generatedAt) <= Date.parse(previous.generatedAt)) {
    throw new Error("stale report block comparison: report timestamp does not advance the current record");
  }
  if (previous) assertReportUpdateAllowed(record.period, operation, record.generatedAt);
  const claimedRecord = await claimCurrentReportTransition(location, previous, record);
  await mkdir(dirname(location), { recursive: true });
  await terminateTornTail(location);
  const event: ReportRecordEvent = {
    eventId: claimedRecord.id,
    type: "report.generated",
    record: claimedRecord,
    ts: claimedRecord.generatedAt,
  };
  await appendFile(location, `${JSON.stringify(event)}\n`, "utf8");
  return { inserted: true, record: claimedRecord };
}

function currentReportRecord(events: ReportRecordEvent[], candidate: ReportRecord): ReportRecord | undefined {
  return events
    .filter(
      (event) =>
        event.type === "report.generated" &&
        event.record &&
        event.record.period.id === candidate.period.id &&
        event.record.templateVersion === candidate.templateVersion,
    )
    .map((event) => event.record!)
    .sort((left, right) => left.generatedAt.localeCompare(right.generatedAt) || left.id.localeCompare(right.id))
    .at(-1);
}

export async function appendReportRecord(
  location: ReportRecordLocation,
  record: ReportRecord,
): Promise<AppendReportRecordResult> {
  return serializeLocationAppend(location, () => appendReportRecordUnlocked(location, record));
}

/** Append one explicitly permitted late operation without replacing prior prose.
 * The generated record is a new version, and the operation tag and optional
 * source remain in its immutable block history. */
export async function appendReportLateUpdate(
  location: ReportRecordLocation,
  current: ReportRecord,
  operation: ReportLateUpdateOperation,
  block: ReportRecordBlock,
  at: string,
  source?: ReportRecordSource,
): Promise<AppendReportRecordResult> {
  assertCanonicalRecord(current);
  assertReportUpdateAllowed(current.period, operation, at);
  if (!block.id.trim() || !block.text.trim()) throw new Error("each late report update requires block id and text");
  requireIsoDate(block.relatedAt, "block relatedAt");
  if (operation === "quote" && !source) throw new Error("quote report update requires a source and date");
  if (operation === "feedback") {
    const events = await readLocationEvents(location);
    if (
      !events.some(
        (event) => event.type === "report.snapshot" && event.snapshot?.record.period.id === current.period.id,
      )
    ) {
      throw new Error("feedback report update requires a finalized snapshot");
    }
  }
  if (source && (!source.id.trim() || !source.uri.trim())) throw new Error("late update source requires id and uri");
  if (source) requireIsoDate(source.observedAt, "source observedAt");
  if (current.blocks.some((item) => item.id === block.id)) throw new Error(`duplicate report block id: ${block.id}`);

  const links = [...new Set([...(block.links ?? []), ...(source ? [source.uri] : [])])].sort();
  const updateBlock: ReportRecordBlock = { ...structuredClone(block), links, updateOperation: operation };
  const blocks = structuredClone(current.blocks);
  if (operation === "banner") {
    blocks.unshift(updateBlock);
  } else if (operation === "event-log") {
    const nextEvent = blocks.findIndex(
      (item) => item.updateOperation === "event-log" && item.relatedAt > updateBlock.relatedAt,
    );
    if (nextEvent < 0) blocks.push(updateBlock);
    else blocks.splice(nextEvent, 0, updateBlock);
  } else {
    blocks.push(updateBlock);
  }
  const sources = [...current.sources];
  if (source) {
    const existing = sources.find((item) => item.id === source.id);
    if (existing && canonicalJson(existing) !== canonicalJson(source)) throw new ReportRecordConflictError(source.id);
    if (!existing) sources.push(structuredClone(source));
  }
  const intent = canonicalJson({ operation, block: updateBlock, source: source ?? null });
  const sourceCursor = `late:${current.id}:${createHash("sha256").update(intent).digest("hex")}`;
  const candidate = makeReportRecord(
    {
      period: current.period,
      asOf: current.asOf,
      templateVersion: current.templateVersion,
      sourceCursor,
      sources,
      blocks,
    },
    at,
  );
  return serializeLocationAppend(location, () =>
    appendReportRecordUnlocked(location, candidate, current.id, operation),
  );
}

/** Read the latest record per stable ID in append order (currently one per key). */
export async function readReportRecords(location: ReportRecordLocation): Promise<ReportRecord[]> {
  const byId = new Map<string, ReportRecord>();
  for (const event of await readLocationEvents(location)) {
    if (event.type === "report.generated" && event.record) byId.set(event.record.id, event.record);
  }
  return [...byId.values()];
}

/** Current projection: latest event-time source-cursor version per period/template. */
export async function readCurrentReportRecords(location: ReportRecordLocation): Promise<ReportRecord[]> {
  return selectCurrentReportRecords(await readReportRecords(location));
}

/** Project current records from one already-read history snapshot. */
export function selectCurrentReportRecords(records: ReportRecord[]): ReportRecord[] {
  const current = new Map<string, ReportRecord>();
  const ordered = [...records].sort((a, b) => a.generatedAt.localeCompare(b.generatedAt) || a.id.localeCompare(b.id));
  for (const record of ordered) {
    current.set(`${record.period.id}\n${record.templateVersion}`, record);
  }
  return [...current.values()];
}

export interface AppendReportSnapshotResult {
  inserted: boolean;
  snapshot: ReportSnapshot;
}

export interface ReportSnapshotCorrection {
  id: string;
  snapshotId: string;
  originalRecordId: string;
  correctedAt: string;
  record: ReportRecord;
}

export interface AppendReportSnapshotCorrectionResult {
  inserted: boolean;
  correction: ReportSnapshotCorrection;
}

/** Freeze a dated view in the same append-only PM-stack log. Existing snapshot
 * IDs are immutable: a repeated identical write is idempotent, changed content
 * is a conflict that must become a separately linked correction record. */
export async function appendReportSnapshot(
  location: ReportRecordLocation,
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
  const event: ReportRecordEvent = {
    eventId: snapshot.id,
    type: "report.snapshot",
    snapshot,
    ts: snapshotAt,
  };
  if (typeof location !== "string") {
    const events = await readLocationEvents(location);
    assertRecordIdContent(events, record);
    await claimRemoteRecord(location, record);
    const result = await appendDocumentEvent(location, event);
    if (result.event.type !== "report.snapshot" || !result.event.snapshot)
      throw new Error("permanent snapshot has an invalid event type");
    return { inserted: result.inserted, snapshot: result.event.snapshot };
  }
  return serializeAppend(location, async () => {
    const events = await readEvents(location);
    assertRecordIdContent(events, record);
    const existing = events.find(
      (event) => event.type === "report.snapshot" && event.snapshot?.id === snapshot.id,
    )?.snapshot;
    if (existing) {
      if (existing.reportRecordId !== snapshot.reportRecordId || !sameRecordContent(existing.record, snapshot.record)) {
        throw new ReportRecordConflictError(snapshot.id);
      }
      return { inserted: false, snapshot: existing };
    }
    await mkdir(dirname(location), { recursive: true });
    await terminateTornTail(location);
    await appendFile(location, `${JSON.stringify(event)}\n`, "utf8");
    return { inserted: true, snapshot };
  });
}

export async function readReportSnapshots(location: ReportRecordLocation): Promise<ReportSnapshot[]> {
  return (await readLocationEvents(location))
    .filter(
      (event): event is ReportRecordEvent & { snapshot: ReportSnapshot } =>
        event.type === "report.snapshot" && !!event.snapshot,
    )
    .map((event) => event.snapshot);
}

/** Append a correction as a new record linked to an immutable snapshot. */
export async function appendReportSnapshotCorrection(
  location: ReportRecordLocation,
  snapshotId: string,
  record: ReportRecord,
  correctedAt: string,
): Promise<AppendReportSnapshotCorrectionResult> {
  assertCanonicalRecord(record);
  requireIsoDate(correctedAt, "correctedAt");
  const correction: ReportSnapshotCorrection = {
    id: `correction:${snapshotId}:${record.id}`,
    snapshotId,
    originalRecordId: "",
    correctedAt,
    record: structuredClone(record),
  };
  if (typeof location !== "string") {
    const events = await readLocationEvents(location);
    const snapshot = events.find(
      (event) => event.type === "report.snapshot" && event.snapshot?.id === snapshotId,
    )?.snapshot;
    if (!snapshot) throw new Error(`snapshot does not exist: ${snapshotId}`);
    if (snapshot.record.period.id !== record.period.id) throw new Error("correction period must match the snapshot");
    correction.originalRecordId = snapshot.reportRecordId;
    assertRecordIdContent(events, record);
    const event: ReportRecordEvent = {
      eventId: correction.id,
      type: "report.snapshot-correction",
      correction,
      ts: correctedAt,
    };
    const result = await appendDocumentEvent(location, event);
    if (result.event.type !== "report.snapshot-correction" || !result.event.correction) {
      throw new Error("permanent snapshot correction has an invalid event type");
    }
    return { inserted: result.inserted, correction: result.event.correction };
  }
  return serializeAppend(location, async () => {
    const events = await readEvents(location);
    const snapshot = events.find(
      (event) => event.type === "report.snapshot" && event.snapshot?.id === snapshotId,
    )?.snapshot;
    if (!snapshot) throw new Error(`snapshot does not exist: ${snapshotId}`);
    if (snapshot.record.period.id !== record.period.id) throw new Error("correction period must match the snapshot");
    correction.originalRecordId = snapshot.reportRecordId;
    assertRecordIdContent(events, record);
    const existing = events.find(
      (event) => event.type === "report.snapshot-correction" && event.correction?.id === correction.id,
    )?.correction;
    if (existing) {
      if (
        existing.snapshotId !== correction.snapshotId ||
        existing.originalRecordId !== correction.originalRecordId ||
        !sameRecordContent(existing.record, correction.record)
      ) {
        throw new ReportRecordConflictError(correction.id);
      }
      return { inserted: false, correction: existing };
    }
    await mkdir(dirname(location), { recursive: true });
    await terminateTornTail(location);
    const event: ReportRecordEvent = {
      eventId: correction.id,
      type: "report.snapshot-correction",
      correction,
      ts: correctedAt,
    };
    await appendFile(location, `${JSON.stringify(event)}\n`, "utf8");
    return { inserted: true, correction };
  });
}

export async function readReportSnapshotCorrections(
  location: ReportRecordLocation,
): Promise<ReportSnapshotCorrection[]> {
  return (await readLocationEvents(location))
    .filter(
      (
        event,
      ): event is ReportRecordEvent & {
        correction: ReportSnapshotCorrection;
      } => event.type === "report.snapshot-correction" && !!event.correction,
    )
    .map((event) => event.correction);
}

/** Resolve a current block conflict by appending a new report version. */
export async function appendReportBlockConflictResolution(
  location: ReportRecordLocation,
  record: ReportRecord,
  blockId: string,
  comparison: ReportBlockComparison,
  strategy: "stored" | "rendered" | "edited" | "combined",
  resolvedAt: string,
  resolvedText?: string,
  comparedRecordId = `${record.id}:comparison`,
): Promise<AppendReportRecordResult> {
  return serializeLocationAppend(location, async () => {
    assertCanonicalRecord(record);
    requireIsoDate(resolvedAt, "resolvedAt");
    const persistedSource = (await readReportRecords(location)).find((candidate) => candidate.id === record.id);
    if (!persistedSource || !sameRecordContent(persistedSource, record)) {
      throw new Error("stale report block comparison: supplied source differs from its persisted record");
    }
    if (!comparison.conflict || !reportBlockConflict(comparison.storedText, comparison.renderedText)) {
      throw new Error("report block comparison does not contain a conflict");
    }
    if (!comparedRecordId.trim() || comparedRecordId === record.id) {
      throw new Error("compared report record is required and must differ from the stored record");
    }

    const canonicalComparison = compareReportBlock(comparison.storedText, comparison.renderedText);
    if (canonicalJson(canonicalComparison) !== canonicalJson(comparison)) {
      throw new Error("invalid report block comparison");
    }
    if ((strategy === "edited" || strategy === "combined") && !resolvedText?.trim()) {
      throw new Error(`${strategy} report block resolution requires non-empty resolved text`);
    }
    if ((strategy === "stored" || strategy === "rendered") && resolvedText !== undefined) {
      throw new Error(`${strategy} report block resolution does not accept resolved text`);
    }
    const text =
      strategy === "stored" ? comparison.storedText : strategy === "rendered" ? comparison.renderedText : resolvedText!;
    const blockIndex = record.blocks.findIndex((block) => block.id === blockId);
    if (blockIndex < 0 || record.blocks[blockIndex].text !== comparison.storedText) {
      throw new Error("stale report block comparison: stored block has changed");
    }

    const cursor = `resolution:${record.id}:${comparedRecordId}:${blockId}:${strategy}:${createHash("sha256").update(text).digest("hex")}`;
    const blocks = structuredClone(record.blocks);
    blocks[blockIndex] = {
      ...blocks[blockIndex],
      text,
      resolution: { sourceRecordId: record.id, comparedRecordId, strategy, resolvedAt },
    };
    const resolvedRecord = makeReportRecord(
      {
        period: record.period,
        asOf: record.asOf,
        templateVersion: record.templateVersion,
        sourceCursor: cursor,
        sources: record.sources,
        blocks,
      },
      resolvedAt,
    );
    const previousResolution = (await readReportRecords(location)).find(
      (candidate) => candidate.id === resolvedRecord.id,
    );
    if (previousResolution) {
      if (!sameResolutionIntent(previousResolution, resolvedRecord, blockId)) {
        throw new ReportRecordConflictError(resolvedRecord.id);
      }
      return { inserted: false, record: previousResolution };
    }

    const existingClaim = await readResolutionSourceClaim(location, record.id);
    if (existingClaim) {
      if (!sameResolutionIntent(existingClaim, resolvedRecord, blockId)) {
        throw new Error("stale report block comparison: another resolution already won");
      }
      return appendReportRecordUnlocked(location, existingClaim, record.id);
    }

    if (Date.parse(resolvedAt) <= Date.parse(persistedSource.generatedAt)) {
      throw new Error("resolvedAt must be later than the persisted source record to preserve causal ordering");
    }
    assertReportUpdateAllowed(persistedSource.period, "edit", resolvedAt);

    const latest = (await readCurrentReportRecords(location)).find(
      (candidate) => candidate.period.id === record.period.id && candidate.templateVersion === record.templateVersion,
    );
    if (!latest || latest.id !== record.id)
      throw new Error("stale report block comparison: report record is no longer current");
    const claimedRecord = await claimResolutionSource(location, record.id, resolvedRecord);
    if (!sameResolutionIntent(claimedRecord, resolvedRecord, blockId)) {
      throw new Error("stale report block comparison: another resolution already won");
    }
    return appendReportRecordUnlocked(location, claimedRecord, record.id);
  });
}

function sameResolutionIntent(left: ReportRecord, right: ReportRecord, blockId: string): boolean {
  const leftBlock = left.blocks.find((block) => block.id === blockId);
  const rightBlock = right.blocks.find((block) => block.id === blockId);
  return (
    left.id === right.id &&
    leftBlock?.text === rightBlock?.text &&
    leftBlock?.resolution?.sourceRecordId === rightBlock?.resolution?.sourceRecordId &&
    leftBlock?.resolution?.comparedRecordId === rightBlock?.resolution?.comparedRecordId &&
    leftBlock?.resolution?.strategy === rightBlock?.resolution?.strategy
  );
}

function assertRecordIdContent(events: ReportRecordEvent[], candidate: ReportRecord): void {
  for (const event of events) {
    const persisted =
      event.type === "report.generated"
        ? event.record
        : event.type === "report.snapshot"
          ? event.snapshot?.record
          : event.correction?.record;
    if (persisted?.id === candidate.id && !sameRecordContent(persisted, candidate)) {
      throw new ReportRecordConflictError(candidate.id);
    }
  }
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
    const keys = Object.keys(object)
      .filter((key) => object[key] !== undefined)
      .sort();
    return `{${keys.map((key) => `${JSON.stringify(key)}:${canonicalJson(object[key])}`).join(",")}}`;
  }
  return JSON.stringify(value) ?? "null";
}

/** Whitespace-only edits are equivalent; any substantive divergence is a conflict. */
export function reportBlockConflict(storedText: string, renderedText: string): boolean {
  const normalize = (text: string) => text.replace(/\s+/g, " ").trim();
  return normalize(storedText) !== normalize(renderedText);
}

export interface ReportBlockDiffHunk {
  before: string;
  after: string;
  beforeLines: string[];
  afterLines: string[];
}

export interface ReportBlockComparison {
  conflict: boolean;
  storedText: string;
  renderedText: string;
  hunks: ReportBlockDiffHunk[];
}

/** Compare both immutable inputs and expose changed line runs for review UI. */
export function compareReportBlock(storedText: string, renderedText: string): ReportBlockComparison {
  if (!reportBlockConflict(storedText, renderedText)) {
    return { conflict: false, storedText, renderedText, hunks: [] };
  }
  const left = storedText.split(/\r?\n/);
  const right = renderedText.split(/\r?\n/);
  // Keep review work bounded for unusually large prose blocks. A single hunk
  // remains truthful when a full line-level alignment would be too expensive.
  if ((left.length + 1) * (right.length + 1) > 200_000) {
    return {
      conflict: true,
      storedText,
      renderedText,
      hunks: [{ before: storedText, after: renderedText, beforeLines: left, afterLines: right }],
    };
  }
  const rows = left.length + 1;
  const cols = right.length + 1;
  const lengths = Array.from({ length: rows }, () => new Uint32Array(cols));
  for (let i = left.length - 1; i >= 0; i--) {
    for (let j = right.length - 1; j >= 0; j--) {
      lengths[i][j] = left[i] === right[j] ? lengths[i + 1][j + 1] + 1 : Math.max(lengths[i + 1][j], lengths[i][j + 1]);
    }
  }
  const hunks: ReportBlockDiffHunk[] = [];
  let before: string[] = [];
  let after: string[] = [];
  const flush = () => {
    if (before.length || after.length) {
      hunks.push({
        before: before.join("\n"),
        after: after.join("\n"),
        beforeLines: [...before],
        afterLines: [...after],
      });
    }
    before = [];
    after = [];
  };
  let i = 0;
  let j = 0;
  while (i < left.length && j < right.length) {
    if (left[i] === right[j]) {
      flush();
      i++;
      j++;
    } else if (lengths[i + 1][j] >= lengths[i][j + 1]) before.push(left[i++]);
    else after.push(right[j++]);
  }
  while (i < left.length) before.push(left[i++]);
  while (j < right.length) after.push(right[j++]);
  flush();
  return { conflict: true, storedText, renderedText, hunks };
}
