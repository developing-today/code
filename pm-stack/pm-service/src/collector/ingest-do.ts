/**
 * IngestDO — one Durable Object per board, keyed by the registry
 * boardId (component A design §1/§3). It serializes collector runs
 * for its board and holds the board's ingest cursors: the ETag /
 * last-snapshot watermark the incremental fetch keys on, and the
 * board's GitHub rate-budget state.
 *
 * Per design §7 invariant 6 the tenant DB is the record and DO
 * state is derived and rebuildable: everything here is cursor and
 * budget bookkeeping — losing it costs one full re-fetch, never
 * data. And per E §2.3 the state holds NO token: tokens are minted
 * per run by the tenancy seam and die with the run; they never pass
 * through this object (the run function receives the fetch closure,
 * not the token).
 *
 * The class is written against a minimal async key/value storage
 * surface so the same code runs on Durable Object storage in the
 * Worker and on the in-memory adapter in tests.
 */
import type { RateBudget } from "./fetch.ts";

/** The async key/value surface of Durable Object storage (subset). */
export interface IngestStorage {
  get<T>(key: string): Promise<T | undefined>;
  put<T>(key: string, value: T): Promise<void>;
}

/** In-memory IngestStorage for tests and local tooling. */
export class MemoryIngestStorage implements IngestStorage {
  private readonly values = new Map<string, unknown>();

  async get<T>(key: string): Promise<T | undefined> {
    return this.values.get(key) as T | undefined;
  }

  async put<T>(key: string, value: T): Promise<void> {
    this.values.set(key, value);
  }

  /** Test/debug view of everything held — used by the no-token pin. */
  snapshot(): Record<string, unknown> {
    return Object.fromEntries(this.values);
  }
}

/** One board's ingest cursors + rate-budget state (IngestDO state). */
export interface IngestState {
  /** ETag of the board's last fetched export; the next run's
   * first-page request is conditional on it. */
  etag: string | null;
  /** Watermark: generatedAt of the last snapshot that landed. */
  lastSnapshotAt: string | null;
  /** Snapshot id of the last landed snapshot, when known. */
  lastSnapshotId: string | null;
  /** Last observed GitHub rate budget for the board's org token. */
  rate: RateBudget;
  /** Outcome of the most recent run, for operator surfaces. */
  lastOutcome: "landed" | "unchanged" | "failed" | "deferred" | null;
  lastRunAt: string | null;
}

const STATE_KEY = "ingest-state";

const EMPTY_STATE: IngestState = {
  etag: null,
  lastSnapshotAt: null,
  lastSnapshotId: null,
  rate: { limit: null, remaining: null, resetAt: null },
  lastOutcome: null,
  lastRunAt: null,
};

export class IngestDO {
  private tail: Promise<unknown> = Promise.resolve();

  constructor(private readonly storage: IngestStorage) {}

  async state(): Promise<IngestState> {
    return (await this.storage.get<IngestState>(STATE_KEY)) ?? { ...EMPTY_STATE };
  }

  /**
   * Serialize one collector run for this board: runs chain on the
   * object's tail promise, so a cron tick that arrives while a run
   * is still in flight queues behind it instead of racing the
   * cursors. A rejected run does not poison the chain.
   */
  runSerialized<T>(run: (state: IngestState) => Promise<{ result: T; next: IngestState }>): Promise<T> {
    const task = async (): Promise<T> => {
      const state = await this.state();
      const { result, next } = await run(state);
      await this.storage.put(STATE_KEY, next);
      return result;
    };
    const result = this.tail.then(task, task);
    this.tail = result.then(
      () => undefined,
      () => undefined,
    );
    return result;
  }
}

/** Is the board's GitHub budget spent until `resetAt` (unix seconds)?
 * A board at zero remaining defers — fail-soft — instead of burning
 * a failing request into the rate wall. Unknown budget never defers. */
export function budgetExhausted(rate: RateBudget, nowIso: string): boolean {
  if (rate.remaining === null || rate.remaining > 0 || rate.resetAt === null) return false;
  return Math.floor(Date.parse(nowIso) / 1000) < rate.resetAt;
}
