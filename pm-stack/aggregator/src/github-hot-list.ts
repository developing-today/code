// Durable consumer for the projector's #5204 GitHub hot list. The
// projector owns webhook intake; this module only sweeps its append-only
// ledger and acknowledges the fixed high-water after activity writes land.
import {
  type ActivitySourceStatus,
  type GithubHotEvent,
  githubHotEventDelivery,
  type IngestionStore,
} from "./activity.ts";
import type { EventsStore } from "./events.ts";

const DEFAULT_CONSUMER = "pm-aggregator-activity-v1";
const DEFAULT_PAGE_SIZE = 200;
const DEFAULT_MAX_PAGES = 100;

interface HotPage {
  events: GithubHotEvent[];
  high_water: number;
  next_cursor: number;
}

class HotListError extends Error {
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

export function unconfiguredGithubStatus(at = new Date()): ActivitySourceStatus {
  const now = at.toISOString();
  return {
    source: "github",
    state: "unconfigured",
    consumer: null,
    cursor: null,
    checkpointCursor: null,
    highWater: null,
    lastAttemptAt: null,
    lastSuccessAt: null,
    updatedAt: now,
    error: "Set PM_PROJECTOR_URL and PM_PROJECTOR_API_TOKEN to enable the hot-list consumer.",
  };
}

export interface GithubHotListConsumerOptions {
  baseUrl: string;
  token: string;
  store: IngestionStore;
  events: Pick<EventsStore, "record">;
  consumer?: string;
  pageSize?: number;
  maxPages?: number;
  fetch?: typeof fetch;
  now?: () => Date;
}

/** Reads one fixed high-water sweep from the projector hot list. A failed
 *  page, append, or checkpoint ack leaves the projector cursor untouched;
 *  the next run safely replays stable `event_id`s through IngestionStore. */
export class GithubHotListConsumer {
  private readonly consumer: string;
  private readonly pageSize: number;
  private readonly maxPages: number;
  private readonly fetcher: typeof fetch;
  private readonly clock: () => Date;
  private inFlight: Promise<ActivitySourceStatus> | null = null;

  constructor(private readonly options: GithubHotListConsumerOptions) {
    this.consumer = options.consumer ?? DEFAULT_CONSUMER;
    this.pageSize = options.pageSize ?? DEFAULT_PAGE_SIZE;
    this.maxPages = options.maxPages ?? DEFAULT_MAX_PAGES;
    this.fetcher = options.fetch ?? fetch;
    this.clock = options.now ?? (() => new Date());
  }

  sync(): Promise<ActivitySourceStatus> {
    if (this.inFlight) return this.inFlight;
    const run = this.syncOnce().finally(() => {
      if (this.inFlight === run) this.inFlight = null;
    });
    this.inFlight = run;
    return run;
  }

  private async syncOnce(): Promise<ActivitySourceStatus> {
    const previous = (await this.options.store.sourceStatuses()).find((row) => row.source === "github");
    const attemptedAt = this.clock().toISOString();
    const base: ActivitySourceStatus = {
      source: "github",
      state: "lagging",
      consumer: this.consumer,
      cursor: previous?.cursor ?? null,
      checkpointCursor: previous?.checkpointCursor ?? null,
      highWater: previous?.highWater ?? null,
      lastAttemptAt: attemptedAt,
      lastSuccessAt: previous?.lastSuccessAt ?? null,
      updatedAt: attemptedAt,
    };
    await this.options.store.setSourceStatus(base);

    let remoteCursor: number | null = null;
    let sweepCursor: number | null = null;
    let sweepHighWater: number | null = null;
    try {
      remoteCursor = await this.getCheckpoint();
      // If a prior run persisted rows but hit its page budget or lost the
      // connection before the remote ack, resume inside that same pinned
      // sweep. IngestionStore has durably recorded every page before this
      // cursor, so continuing here cannot skip an unpersisted activity.
      let cursor = remoteCursor;
      let highWater: number | null = null;
      if (
        previous !== undefined &&
        previous.checkpointCursor === remoteCursor &&
        previous.cursor !== null &&
        previous.cursor > remoteCursor &&
        previous.highWater !== null
      ) {
        cursor = previous.cursor;
        highWater = previous.highWater;
      }
      sweepCursor = cursor;
      sweepHighWater = highWater;
      await this.options.store.setSourceStatus({
        ...base,
        cursor,
        checkpointCursor: remoteCursor,
        highWater,
        updatedAt: this.clock().toISOString(),
      });
      let pages = 0;
      let completed = false;

      while (pages < this.maxPages) {
        const page = await this.getPage(cursor, highWater);
        pages++;
        if (highWater === null) highWater = page.high_water;
        sweepHighWater = highWater;
        if (page.high_water !== highWater) throw new HotListError("hot_list_high_water_changed", "unrecoverable");
        if (remoteCursor > highWater) throw new HotListError("checkpoint_past_hot_list_high_water", "unrecoverable");

        let priorSeq = cursor;
        for (const event of page.events) {
          if (event.seq <= priorSeq || event.seq > highWater) {
            throw new HotListError("hot_list_page_sequence_invalid", "unrecoverable");
          }
          priorSeq = event.seq;
        }
        const expectedNext = page.events.at(-1)?.seq ?? cursor;
        if (page.next_cursor !== expectedNext) throw new HotListError("hot_list_next_cursor_invalid", "unrecoverable");
        if (page.events.length === 0 && cursor < highWater) {
          // `high_water` is the maximum existing row seq, so an empty
          // page before that cursor is reached would skip source rows.
          throw new HotListError("hot_list_cursor_stalled_before_high_water", "unrecoverable");
        }

        if (page.events.length) {
          const deliveries = page.events.map(githubHotEventDelivery);
          const ingested = await this.options.store.ingest(this.options.events, deliveries);
          if (ingested.failed > 0) throw new HotListError(`activity_append_failed:${ingested.failed}`);
          cursor = page.next_cursor;
          sweepCursor = cursor;
        }

        await this.options.store.setSourceStatus({
          ...base,
          cursor,
          checkpointCursor: remoteCursor,
          highWater,
          updatedAt: this.clock().toISOString(),
        });
        if (page.events.length < this.pageSize && cursor < highWater) {
          throw new HotListError("hot_list_page_ended_before_high_water", "unrecoverable");
        }
        if (cursor >= highWater) {
          completed = true;
          break;
        }
      }

      if (!completed || highWater === null) {
        const status: ActivitySourceStatus = {
          ...base,
          state: "lagging",
          cursor: sweepCursor,
          checkpointCursor: remoteCursor,
          highWater,
          error: `sweep_exceeded_${this.maxPages}_pages`,
          updatedAt: this.clock().toISOString(),
        };
        await this.openGap(previous, status.updatedAt);
        await this.options.store.setSourceStatus(status);
        return status;
      }

      const ackedCursor = await this.acknowledge(cursor, highWater);
      const completedAt = this.clock().toISOString();
      const state = ackedCursor < highWater ? "lagging" : "current";
      const status: ActivitySourceStatus = {
        source: "github",
        state,
        consumer: this.consumer,
        cursor: ackedCursor,
        checkpointCursor: ackedCursor,
        highWater,
        lastAttemptAt: attemptedAt,
        lastSuccessAt: completedAt,
        updatedAt: completedAt,
        ...(state === "lagging" ? { error: "remote_checkpoint_ahead_of_completed_sweep" } : {}),
      };
      if (state === "current") await this.resolveOpenGaps(completedAt);
      else await this.openGap(previous, completedAt);
      await this.options.store.setSourceStatus(status);
      return status;
    } catch (error) {
      const disposition = error instanceof HotListError ? error.disposition : "outage";
      const failedAt = this.clock().toISOString();
      const message = errorMessage(error);
      const status: ActivitySourceStatus = {
        ...base,
        state: disposition,
        cursor: sweepCursor ?? base.cursor,
        checkpointCursor: remoteCursor ?? base.checkpointCursor,
        highWater: sweepHighWater ?? base.highWater,
        updatedAt: failedAt,
        error: message,
      };
      const gap = await this.openGap(previous, failedAt);
      if (disposition === "unrecoverable") await this.options.store.resolveGap(gap.id, "unrecoverable", failedAt);
      await this.options.store.setSourceStatus(status);
      console.error(`[activity] WARNING: GitHub hot-list sync ${disposition}: ${message}`);
      return status;
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
      throw new HotListError(`projector_request_failed:${errorMessage(error)}`);
    }
    if (!response.ok) throw new HotListError(`projector_http_${response.status}`);
    try {
      return await response.json();
    } catch {
      throw new HotListError("projector_response_invalid_json", "unrecoverable");
    }
  }

  private async getCheckpoint(): Promise<number> {
    const query = new URLSearchParams({ consumer: this.consumer });
    const value = await this.json(`/hot/checkpoints?${query}`);
    if (typeof value !== "object" || value === null || !isCursor((value as Record<string, unknown>).cursor)) {
      throw new HotListError("projector_checkpoint_shape_invalid", "unrecoverable");
    }
    return (value as { cursor: number }).cursor;
  }

  private async getPage(after: number, highWater: number | null): Promise<HotPage> {
    const query = new URLSearchParams({ after: String(after), limit: String(this.pageSize) });
    if (highWater !== null) query.set("high_water", String(highWater));
    const value = await this.json(`/hot?${query}`);
    if (!isHotPage(value)) throw new HotListError("projector_hot_page_shape_invalid", "unrecoverable");
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
      throw new HotListError("projector_ack_shape_invalid", "unrecoverable");
    }
    const acked = (value as { cursor: number }).cursor;
    if (acked < cursor) throw new HotListError("projector_checkpoint_did_not_advance", "unrecoverable");
    return acked;
  }

  private async openGap(previous: ActivitySourceStatus | undefined, to: string) {
    const existing = (await this.options.store.gaps()).find((gap) => gap.source === "github" && gap.status === "open");
    const from = existing?.from ?? previous?.lastSuccessAt ?? previous?.lastAttemptAt ?? to;
    return this.options.store.recordGap("github", from, to);
  }

  private async resolveOpenGaps(at: string): Promise<void> {
    for (const gap of await this.options.store.gaps()) {
      if (gap.source === "github" && gap.status === "open")
        await this.options.store.resolveGap(gap.id, "backfilled", at);
    }
  }
}
