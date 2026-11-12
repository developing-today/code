import { randomUUID } from "node:crypto";
import { mkdir, open, readFile, rename, rm } from "node:fs/promises";
import { dirname } from "node:path";
import { parseIsoDate } from "./validate.ts";

export type FreshnessSource = "github" | "kaneo" | "fleet-feed" | "orchestration" | "reconcile";
export type FreshnessReason = "source-event" | "reconcile" | "recovery";

export interface FreshnessInput {
  eventId: string;
  source: FreshnessSource;
  scope: string[];
  occurredAt: string | null;
  reason: FreshnessReason;
}

export interface FreshnessNotice extends FreshnessInput {
  schemaVersion: 1;
  sequence: number;
  observedAt: string;
}

interface JournalFile {
  version: 1;
  cursor: number;
  events: FreshnessNotice[];
}

interface OutboxEntry {
  input: FreshnessInput;
  observedAt: string;
}

const INPUT_KEYS = ["eventId", "source", "scope", "occurredAt", "reason"] as const;
const NOTICE_KEYS = [...INPUT_KEYS, "schemaVersion", "sequence", "observedAt"] as const;
const JOURNAL_KEYS = ["version", "cursor", "events"] as const;
const OUTBOX_KEYS = ["input", "observedAt"] as const;
const SOURCES = new Set<FreshnessSource>(["github", "kaneo", "fleet-feed", "orchestration", "reconcile"]);
const REASONS = new Set<FreshnessReason>(["source-event", "reconcile", "recovery"]);
const MAX_NOTICES = 10_000;
const MAX_OUTBOX_ENTRIES = 10_000;
const RETENTION_MS = 24 * 60 * 60 * 1000;

function exactRecord(value: unknown, keys: readonly string[]): value is Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return false;
  const prototype = Object.getPrototypeOf(value);
  if (prototype !== Object.prototype && prototype !== null) return false;
  const actual = Reflect.ownKeys(value);
  if (actual.length !== keys.length || actual.some((key) => typeof key !== "string" || !keys.includes(key)))
    return false;
  return keys.every((key) => {
    const descriptor = Object.getOwnPropertyDescriptor(value, key);
    return descriptor !== undefined && descriptor.enumerable && Object.hasOwn(descriptor, "value");
  });
}

function normalizeInput(value: unknown): FreshnessInput {
  if (!exactRecord(value, INPUT_KEYS)) throw new Error("invalid freshness input shape");
  const row = value as Record<string, unknown>;
  if (typeof row.eventId !== "string" || row.eventId.length === 0 || row.eventId.length > 512) {
    throw new Error("invalid freshness eventId");
  }
  if (typeof row.source !== "string" || !SOURCES.has(row.source as FreshnessSource)) {
    throw new Error("invalid freshness source");
  }
  if (
    !Array.isArray(row.scope) ||
    row.scope.length === 0 ||
    row.scope.length > 100 ||
    row.scope.some((item) => typeof item !== "string" || item.length === 0 || item.length > 256)
  ) {
    throw new Error("invalid freshness scope");
  }
  if (row.occurredAt !== null && (typeof row.occurredAt !== "string" || parseIsoDate(row.occurredAt) === null)) {
    throw new Error("invalid freshness occurredAt");
  }
  if (typeof row.reason !== "string" || !REASONS.has(row.reason as FreshnessReason)) {
    throw new Error("invalid freshness reason");
  }
  return {
    eventId: row.eventId,
    source: row.source as FreshnessSource,
    scope: [...row.scope] as string[],
    occurredAt: row.occurredAt as string | null,
    reason: row.reason as FreshnessReason,
  };
}

function validObservedAt(value: unknown): value is string {
  return typeof value === "string" && parseIsoDate(value) !== null;
}

function normalizeNotice(value: unknown): FreshnessNotice {
  if (!exactRecord(value, NOTICE_KEYS)) throw new Error("invalid freshness notice shape");
  const row = value as Record<string, unknown>;
  const input = normalizeInput({
    eventId: row.eventId,
    source: row.source,
    scope: row.scope,
    occurredAt: row.occurredAt,
    reason: row.reason,
  });
  if (row.schemaVersion !== 1 || !Number.isSafeInteger(row.sequence) || (row.sequence as number) < 1) {
    throw new Error("invalid freshness notice sequence");
  }
  if (!validObservedAt(row.observedAt)) throw new Error("invalid freshness observedAt");
  return { ...input, schemaVersion: 1, sequence: row.sequence as number, observedAt: row.observedAt };
}

function normalizeJournal(value: unknown): JournalFile {
  if (!exactRecord(value, JOURNAL_KEYS)) throw new Error("invalid freshness journal shape");
  const row = value as Record<string, unknown>;
  if (
    row.version !== 1 ||
    !Number.isSafeInteger(row.cursor) ||
    (row.cursor as number) < 0 ||
    !Array.isArray(row.events)
  ) {
    throw new Error("invalid freshness journal metadata");
  }
  const events = row.events.map(normalizeNotice);
  let previousSequence = 0;
  const ids = new Set<string>();
  for (const event of events) {
    if (event.sequence <= previousSequence || event.sequence > (row.cursor as number) || ids.has(event.eventId)) {
      throw new Error("invalid freshness journal ordering");
    }
    previousSequence = event.sequence;
    ids.add(event.eventId);
  }
  return { version: 1, cursor: row.cursor as number, events };
}

function normalizeOutboxEntry(value: unknown): OutboxEntry {
  if (!exactRecord(value, OUTBOX_KEYS)) throw new Error("invalid freshness outbox shape");
  const row = value as Record<string, unknown>;
  if (!validObservedAt(row.observedAt)) throw new Error("invalid freshness outbox observedAt");
  return { input: normalizeInput(row.input), observedAt: row.observedAt };
}

function retention(events: FreshnessNotice[], now: number): FreshnessNotice[] {
  const cutoff = now - RETENTION_MS;
  return events
    .filter((event) => (parseIsoDate(event.observedAt)?.getTime() ?? -Infinity) >= cutoff)
    .slice(-MAX_NOTICES);
}

function makeNotice(input: FreshnessInput, sequence: number, observedAt: string): FreshnessNotice {
  return { ...input, scope: [...input.scope], schemaVersion: 1, sequence, observedAt };
}

function copyNotice(notice: FreshnessNotice): FreshnessNotice {
  return { ...notice, scope: [...notice.scope] };
}

export class FreshnessStore {
  private events: FreshnessNotice[] = [];
  private cursor = 0;
  private pending: OutboxEntry[] = [];
  private loaded = false;
  private operationChain: Promise<void> = Promise.resolve();
  private writeSequence = 0;

  constructor(private readonly file: string) {}

  publish(input: FreshnessInput): Promise<{ notice: FreshnessNotice; duplicate: boolean; pending: boolean }> {
    return this.serialize(async () => {
      const normalized = normalizeInput(input);
      await this.load();
      const now = new Date();
      await this.flush(now);

      const existing = this.events.find((event) => event.eventId === normalized.eventId);
      if (existing) return { notice: copyNotice(existing), duplicate: true, pending: false };
      const queued = this.pending.find((entry) => entry.input.eventId === normalized.eventId);
      if (queued) return { notice: makeNotice(queued.input, 0, queued.observedAt), duplicate: true, pending: true };

      const observedAt = now.toISOString();
      // Preserve outbox order: a later event cannot obtain a cursor while an
      // earlier accepted event is still waiting for journal persistence.
      if (this.pending.length > 0) {
        await this.appendOutbox({ input: normalized, observedAt });
        this.pending.push({ input: normalized, observedAt });
        return { notice: makeNotice(normalized, 0, observedAt), duplicate: false, pending: true };
      }

      if (this.cursor >= Number.MAX_SAFE_INTEGER) throw new Error("freshness sequence exhausted");
      const candidateCursor = this.cursor + 1;
      const candidate = retention(
        [...retention(this.events, now.getTime()), makeNotice(normalized, candidateCursor, observedAt)],
        now.getTime(),
      );
      try {
        await this.persist(candidate, candidateCursor);
        this.events = candidate;
        this.cursor = candidateCursor;
        return {
          notice: copyNotice(candidate.find((event) => event.sequence === candidateCursor)!),
          duplicate: false,
          pending: false,
        };
      } catch {
        await this.appendOutbox({ input: normalized, observedAt });
        this.pending.push({ input: normalized, observedAt });
        // Sequence zero is reserved for this unsequenced acknowledgement.
        return { notice: makeNotice(normalized, 0, observedAt), duplicate: false, pending: true };
      }
    });
  }

  listAfter(
    after: number,
    scope?: string,
  ): Promise<{ cursor: number; resetRequired: boolean; events: FreshnessNotice[] }> {
    return this.serialize(async () => {
      if (!Number.isSafeInteger(after) || after < 0) throw new Error("after must be a non-negative safe integer");
      if (scope !== undefined && typeof scope !== "string") throw new Error("scope must be a string");
      await this.load();
      const now = new Date();
      await this.flush(now);
      const visible = retention(this.events, now.getTime());
      const firstSequence = visible[0]?.sequence;
      const resetRequired =
        after > this.cursor || (after < this.cursor && (firstSequence === undefined || after < firstSequence - 1));
      return {
        cursor: this.cursor,
        resetRequired,
        events: visible
          .filter((event) => event.sequence > after && (scope === undefined || event.scope.includes(scope)))
          .map(copyNotice),
      };
    });
  }

  private serialize<T>(operation: () => Promise<T>): Promise<T> {
    const run = this.operationChain.then(operation, operation);
    this.operationChain = run.then(
      () => undefined,
      () => undefined,
    );
    return run;
  }

  private async load(): Promise<void> {
    if (this.loaded) return;
    let raw: string | null = null;
    try {
      raw = await readFile(this.file, "utf8");
    } catch (error) {
      if ((error as { code?: unknown })?.code !== "ENOENT") throw new Error("cannot read freshness journal");
    }

    let journal: JournalFile = { version: 1, cursor: 0, events: [] };
    if (raw !== null) {
      try {
        journal = normalizeJournal(JSON.parse(raw) as unknown);
      } catch {
        throw new Error("invalid freshness journal; original file left untouched");
      }
    }

    let outboxRaw: string | null = null;
    try {
      outboxRaw = await readFile(this.outboxFile, "utf8");
    } catch (error) {
      if ((error as { code?: unknown })?.code !== "ENOENT") throw new Error("cannot read freshness outbox");
    }
    const pending = outboxRaw === null ? [] : this.parseOutbox(outboxRaw);
    this.events = journal.events;
    this.cursor = journal.cursor;
    this.pending = pending;
    this.loaded = true;
  }

  private parseOutbox(raw: string): OutboxEntry[] {
    if (raw === "") return [];
    const lines = raw.endsWith("\n") ? raw.slice(0, -1).split("\n") : raw.split("\n");
    const seen = new Set<string>();
    const entries: OutboxEntry[] = [];
    for (const line of lines) {
      if (line.length === 0) throw new Error("invalid freshness outbox; original file left untouched");
      let entry: OutboxEntry;
      try {
        entry = normalizeOutboxEntry(JSON.parse(line) as unknown);
      } catch {
        throw new Error("invalid freshness outbox; original file left untouched");
      }
      if (!seen.has(entry.input.eventId)) {
        if (entries.length >= MAX_OUTBOX_ENTRIES) {
          throw new Error("freshness outbox exceeds its configured limit; original file left untouched");
        }
        seen.add(entry.input.eventId);
        entries.push(entry);
      }
    }
    return entries;
  }

  private async flush(now: Date): Promise<boolean> {
    const retained = retention(this.events, now.getTime());
    const changed = retained.length !== this.events.length;
    if (this.pending.length === 0 && !changed) return true;

    // Include journal ids that retention is about to remove: an outbox line
    // may be a replay after the journal rename succeeded but outbox cleanup did not.
    const ids = new Set(this.events.map((event) => event.eventId));
    let candidateCursor = this.cursor;
    const pendingNotices: FreshnessNotice[] = [];
    for (const entry of this.pending) {
      if (ids.has(entry.input.eventId)) continue;
      if (candidateCursor >= Number.MAX_SAFE_INTEGER) throw new Error("freshness sequence exhausted");
      pendingNotices.push(makeNotice(entry.input, ++candidateCursor, entry.observedAt));
      ids.add(entry.input.eventId);
    }
    // Pending events must reach the durable journal before their outbox is
    // removed, even if recovery happens after the normal replay retention.
    // Make room by evicting older retained journal rows, never new outbox rows.
    const retainedLimit = Math.max(0, MAX_NOTICES - pendingNotices.length);
    const retainedEvents = retainedLimit === 0 ? [] : retained.slice(-retainedLimit);
    const candidate = [...retainedEvents, ...pendingNotices];
    try {
      await this.persist(candidate, candidateCursor);
    } catch {
      return false;
    }
    this.events = candidate;
    this.cursor = candidateCursor;
    if (this.pending.length > 0) {
      try {
        await rm(this.outboxFile);
        this.pending = [];
      } catch (error) {
        if ((error as { code?: unknown })?.code === "ENOENT") this.pending = [];
      }
    }
    return true;
  }

  private get outboxFile(): string {
    return `${this.file}.outbox.jsonl`;
  }

  private async appendOutbox(entry: OutboxEntry): Promise<void> {
    if (this.pending.length >= MAX_OUTBOX_ENTRIES) {
      throw new Error("freshness outbox is full; retry after journal recovery");
    }
    const directory = dirname(this.outboxFile);
    await mkdir(directory, { recursive: true });
    const handle = await open(this.outboxFile, "a", 0o600);
    try {
      await handle.writeFile(`${JSON.stringify(entry)}\n`);
      await handle.sync();
    } finally {
      await handle.close();
    }
    const directoryHandle = await open(directory, "r");
    try {
      await directoryHandle.sync();
    } finally {
      await directoryHandle.close();
    }
  }

  private async persist(events: FreshnessNotice[], cursor: number): Promise<void> {
    await mkdir(dirname(this.file), { recursive: true });
    const temporary = `${this.file}.tmp-${process.pid}-${++this.writeSequence}-${randomUUID()}`;
    let handle: Awaited<ReturnType<typeof open>> | null = null;
    try {
      handle = await open(temporary, "wx", 0o600);
      const journal: JournalFile = { version: 1, cursor, events };
      await handle.writeFile(JSON.stringify(journal));
      await handle.sync();
      await handle.close();
      handle = null;
      await rename(temporary, this.file);
      const directoryHandle = await open(dirname(this.file), "r");
      try {
        await directoryHandle.sync();
      } finally {
        await directoryHandle.close();
      }
    } catch (error) {
      if (handle) await handle.close().catch(() => {});
      await rm(temporary, { force: true }).catch(() => {});
      throw error;
    }
  }
}
