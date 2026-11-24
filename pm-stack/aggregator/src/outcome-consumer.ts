// Live cadence wiring for outcome attribution (#10910 remainder of the
// #2072 slice-4 spec): a consumer on the aggregator's sweep, next to
// GithubHotListConsumer, that projects the projector's #5204 hot list
// into the append-only outcome-fact ledger (outcome-attribution.ts).
//
// The two consumers are independent on purpose: the activity consumer
// owns activity ingestion under its own projector checkpoint, while
// this consumer's checkpoint acknowledges outcome-fact durability. The
// OutcomeFactStore cursor is this consumer's resume point — it advances
// only after a sweep page's facts are durably written — and fact-id
// dedupe makes any replay of an unacknowledged sweep a no-op, so the
// projector checkpoint is acked only when the ledger covers the full
// fixed high-water.
import { readFile } from "node:fs/promises";
import type { GithubHotEvent } from "./activity.ts";
import { OutcomeFactStore, type OutcomeBinding } from "./outcome-attribution.ts";

const DEFAULT_CONSUMER = "pm-aggregator-outcome-v1";
const DEFAULT_PAGE_SIZE = 200;
const DEFAULT_MAX_PAGES = 100;

interface HotPage {
  events: GithubHotEvent[];
  high_water: number;
  next_cursor: number;
}

class OutcomeHotListError extends Error {
  constructor(
    message: string,
    readonly disposition: "outage" | "unrecoverable" = "outage",
  ) {
    super(message);
  }
}

function isCursor(value: unknown): value is number {
  return Number.isSafeInteger(value) && Number(value) >= 0;
}

function isNullableString(value: unknown): value is string | null {
  return value === null || typeof value === "string";
}

function isHotEvent(value: unknown): value is GithubHotEvent {
  if (typeof value !== "object" || value === null) return false;
  const event = value as Record<string, unknown>;
  return (
    isCursor(event.seq) &&
    typeof event.event_id === "string" &&
    isNullableString(event.delivery_id) &&
    typeof event.repo === "string" &&
    typeof event.event === "string" &&
    isNullableString(event.action) &&
    isNullableString(event.ref) &&
    isNullableString(event.actor) &&
    isNullableString(event.source_url) &&
    isNullableString(event.source_timestamp) &&
    typeof event.received_at === "string"
  );
}

function isHotPage(value: unknown): value is HotPage {
  if (typeof value !== "object" || value === null) return false;
  const page = value as Record<string, unknown>;
  return (
    Array.isArray(page.events) &&
    page.events.every(isHotEvent) &&
    isCursor(page.high_water) &&
    isCursor(page.next_cursor)
  );
}

function errorMessage(error: unknown): string {
  return (error instanceof Error ? error.message : String(error)).slice(0, 500);
}

/** Authoritative joins between hot-list events and the delegated work
 *  that produced them. Returning no binding for an event is a
 *  statement, not an error: the event's fact lands explicitly
 *  unattributed (`no-binding`). A source must never guess a join from
 *  the GitHub actor, a branch name, or time proximity (#10910 spec). */
export interface OutcomeBindingSource {
  resolve(events: readonly GithubHotEvent[]): Promise<readonly OutcomeBinding[]>;
}

const BINDING_STRING_FIELDS = [
  "taskId",
  "jobId",
  "sessionId",
  "rootSessionId",
  "parentSessionId",
  "providerID",
  "modelID",
] as const;

function isBindingRecord(value: unknown): value is OutcomeBinding {
  if (typeof value !== "object" || value === null) return false;
  const record = value as Record<string, unknown>;
  if (typeof record.eventId !== "string" || record.eventId.trim() === "") return false;
  if (typeof record.method !== "string" || record.method.trim() === "") return false;
  return BINDING_STRING_FIELDS.every(
    (field) => record[field] === undefined || record[field] === null || typeof record[field] === "string",
  );
}

/** File-backed binding records: one JSON object per line in the
 *  OutcomeBinding shape, appended by whatever produces queue/session
 *  binding records (queue claims, session bindings, schedule records).
 *  A missing file is an empty source — every fact lands unattributed,
 *  honestly. A corrupt line is an explicit
 *  `outcome_binding_source_corrupt` error: a broken binding feed fails
 *  the sweep visibly rather than silently rendering as "no bindings
 *  exist". */
export class FileOutcomeBindingSource implements OutcomeBindingSource {
  constructor(private readonly file: string) {}

  async resolve(events: readonly GithubHotEvent[]): Promise<readonly OutcomeBinding[]> {
    let text: string;
    try {
      text = await readFile(this.file, "utf8");
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") return [];
      throw error;
    }
    const wanted = new Set(events.map((event) => event.event_id.trim()));
    const bindings: OutcomeBinding[] = [];
    for (const line of text.split("\n")) {
      if (!line.trim()) continue;
      let parsed: unknown;
      try {
        parsed = JSON.parse(line);
      } catch {
        throw new Error("outcome_binding_source_corrupt");
      }
      if (!isBindingRecord(parsed)) throw new Error("outcome_binding_source_corrupt");
      if (wanted.has(parsed.eventId.trim())) bindings.push(parsed);
    }
    return bindings;
  }
}

export interface OutcomeSyncResult {
  state: "current" | "lagging" | "outage" | "unrecoverable";
  consumer: string;
  /** Outcome-ledger cursor after this run (durable facts frontier). */
  cursor: number;
  /** Projector-acknowledged cursor for this consumer, when known. */
  checkpointCursor: number | null;
  highWater: number | null;
  /** Facts newly appended by this run (0 on a pure replay). */
  appended: number;
  error: string | null;
}

export interface GithubOutcomeConsumerOptions {
  baseUrl: string;
  token: string;
  store: OutcomeFactStore;
  bindings: OutcomeBindingSource;
  consumer?: string;
  pageSize?: number;
  maxPages?: number;
  fetch?: typeof fetch;
}

/** Sweeps one fixed high-water from the projector hot list into the
 *  outcome-fact ledger. A failed page, binding read, or fact append
 *  leaves both cursors untouched (the ledger cursor covers only durable
 *  facts) and records a source-gap fact, so the gap is visible in the
 *  ledger instead of rendering as zero activity. */
export class GithubOutcomeConsumer {
  private readonly consumer: string;
  private readonly pageSize: number;
  private readonly maxPages: number;
  private readonly fetcher: typeof fetch;
  private inFlight: Promise<OutcomeSyncResult> | null = null;

  constructor(private readonly options: GithubOutcomeConsumerOptions) {
    this.consumer = options.consumer ?? DEFAULT_CONSUMER;
    this.pageSize = options.pageSize ?? DEFAULT_PAGE_SIZE;
    this.maxPages = options.maxPages ?? DEFAULT_MAX_PAGES;
    this.fetcher = options.fetch ?? fetch;
  }

  sync(): Promise<OutcomeSyncResult> {
    if (this.inFlight) return this.inFlight;
    const run = this.syncOnce().finally(() => {
      if (this.inFlight === run) this.inFlight = null;
    });
    this.inFlight = run;
    return run;
  }

  private async syncOnce(): Promise<OutcomeSyncResult> {
    let cursor = 0;
    let checkpointCursor: number | null = null;
    let highWater: number | null = null;
    let appended = 0;
    try {
      cursor = await this.options.store.cursor();
      checkpointCursor = await this.getCheckpoint();
      // The ledger cursor is the resume point: it never exceeds the
      // durable facts, and replay below the projector checkpoint would
      // only re-fetch pages whose facts already dedupe to no-ops.
      let pages = 0;
      let completed = false;
      while (pages < this.maxPages) {
        const page = await this.getPage(cursor, highWater);
        pages++;
        if (highWater === null) highWater = page.high_water;
        if (page.high_water !== highWater) {
          throw new OutcomeHotListError("hot_list_high_water_changed", "unrecoverable");
        }
        if (checkpointCursor > highWater) {
          throw new OutcomeHotListError("checkpoint_past_hot_list_high_water", "unrecoverable");
        }
        let priorSeq = cursor;
        for (const event of page.events) {
          if (event.seq <= priorSeq || event.seq > highWater) {
            throw new OutcomeHotListError("hot_list_page_sequence_invalid", "unrecoverable");
          }
          priorSeq = event.seq;
        }
        const expectedNext = page.events.at(-1)?.seq ?? cursor;
        if (page.next_cursor !== expectedNext) {
          throw new OutcomeHotListError("hot_list_next_cursor_invalid", "unrecoverable");
        }
        if (page.events.length === 0 && cursor < highWater) {
          throw new OutcomeHotListError("hot_list_cursor_stalled_before_high_water", "unrecoverable");
        }
        if (page.events.length) {
          const bindings = await this.options.bindings.resolve(page.events);
          const result = await this.options.store.applySweep({
            sourceStatus: "ok",
            events: page.events,
            bindings,
          });
          appended += result.appended;
          cursor = result.cursor;
        }
        if (page.events.length < this.pageSize && cursor < highWater) {
          throw new OutcomeHotListError("hot_list_page_ended_before_high_water", "unrecoverable");
        }
        if (cursor >= highWater) {
          completed = true;
          break;
        }
      }
      if (!completed || highWater === null) {
        return {
          state: "lagging",
          consumer: this.consumer,
          cursor,
          checkpointCursor,
          highWater,
          appended,
          error: `sweep_exceeded_${this.maxPages}_pages`,
        };
      }
      const ackedCursor = await this.acknowledge(cursor, highWater);
      checkpointCursor = ackedCursor;
      const state = ackedCursor < highWater ? "lagging" : "current";
      return {
        state,
        consumer: this.consumer,
        cursor,
        checkpointCursor,
        highWater,
        appended,
        error: state === "lagging" ? "remote_checkpoint_ahead_of_completed_sweep" : null,
      };
    } catch (error) {
      const disposition = error instanceof OutcomeHotListError ? error.disposition : "outage";
      const message = errorMessage(error);
      // Record the gap in the ledger itself (a source-gap fact, deduped
      // by fact id); the cursor does not move on a failed sweep. A
      // corrupt ledger throws here too — that failure must surface, not
      // be swallowed behind the original error.
      try {
        await this.options.store.applySweep({ sourceStatus: "error", events: [] });
      } catch {
        // The store's own error is the more fundamental one; the run is
        // already failing, and the next tick retries against the store.
      }
      return {
        state: disposition,
        consumer: this.consumer,
        cursor,
        checkpointCursor,
        highWater,
        appended,
        error: message,
      };
    }
  }

  private endpoint(path: string): string {
    return `${this.options.baseUrl.replace(/\/+$/, "")}${path}`;
  }

  private async json(path: string, init?: RequestInit): Promise<unknown> {
    let response: Response;
    try {
      response = await this.fetcher(this.endpoint(path), {
        ...init,
        headers: {
          Accept: "application/json",
          ...(init?.headers ?? {}),
        },
        signal: init?.signal ?? AbortSignal.timeout(8_000),
      });
    } catch (error) {
      throw new OutcomeHotListError(`projector_request_failed:${errorMessage(error)}`);
    }
    if (!response.ok) throw new OutcomeHotListError(`projector_http_${response.status}`);
    try {
      return await response.json();
    } catch {
      throw new OutcomeHotListError("projector_response_invalid_json", "unrecoverable");
    }
  }

  private async getCheckpoint(): Promise<number> {
    const query = new URLSearchParams({ consumer: this.consumer });
    const value = await this.json(`/hot/checkpoints?${query}`);
    if (typeof value !== "object" || value === null || !isCursor((value as Record<string, unknown>).cursor)) {
      throw new OutcomeHotListError("projector_checkpoint_shape_invalid", "unrecoverable");
    }
    return (value as { cursor: number }).cursor;
  }

  private async getPage(after: number, highWater: number | null): Promise<HotPage> {
    const query = new URLSearchParams({ after: String(after), limit: String(this.pageSize) });
    if (highWater !== null) query.set("high_water", String(highWater));
    const value = await this.json(`/hot?${query}`);
    if (!isHotPage(value)) throw new OutcomeHotListError("projector_hot_page_shape_invalid", "unrecoverable");
    return value;
  }

  private async acknowledge(cursor: number, highWater: number): Promise<number> {
    const value = await this.json("/hot/checkpoints", {
      method: "POST",
      headers: {
        Authorization: `Bearer ${this.options.token}`,
        "Content-Type": "application/json",
      },
      body: JSON.stringify({ consumer: this.consumer, cursor, highWater }),
    });
    if (typeof value !== "object" || value === null || !isCursor((value as Record<string, unknown>).cursor)) {
      throw new OutcomeHotListError("projector_ack_shape_invalid", "unrecoverable");
    }
    const acked = (value as { cursor: number }).cursor;
    if (acked < cursor) throw new OutcomeHotListError("projector_checkpoint_did_not_advance", "unrecoverable");
    return acked;
  }
}
