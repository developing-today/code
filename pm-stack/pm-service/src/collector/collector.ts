/**
 * The hosted collector (component A design §3): a Cron-Trigger run
 * that ingests every board in the tenant DB's registry into the
 * tenant DB, fail-soft per board.
 *
 * Per run, per board (serialized through the board's IngestDO):
 * the tenancy seam mints a per-org App installation token for the
 * board's org — minted per run, used for that board's fetch, and
 * dropped; no token value is stored in the tenant DB, in DO state,
 * or anywhere else (E §2.3, test-pinned). A board whose budget is
 * spent defers; a board whose fetch fails records an ingest-failure
 * finding and keeps its previous pool rows; neither touches the
 * other boards. Snapshots are collected first and landed after
 * epic/spawn resolution over the MERGED pool, so cross-board
 * ancestry resolves exactly as it does on the laptop edition.
 *
 * Everything external is injected — tenant DB (SqlDb), token
 * provider, fetch, IngestDO factory, clock — so the whole run is
 * exercised in tests with no network and no Workers runtime. The
 * Worker entrypoint binds these to the real tenancy Worker token
 * function, the global fetch, and Durable Object namespaces.
 */
import type { SqlDb } from "../tenant-store/migrate.ts";
import type { BoardDef } from "../tenant-store/schema.ts";
import { fetchBoardSnapshot, type CollectorFetch, type RateBudget } from "./fetch.ts";
import { budgetExhausted, IngestDO, type IngestState } from "./ingest-do.ts";
import { resolveEpicRefs, resolveSpawnRollups, type ProjectAudit, type ProjectPoolItem } from "./pool.ts";
import { landSnapshot, loadRegistry, recordIngestFinding } from "./store.ts";

/**
 * The tenancy seam (design §3): mint a per-org GitHub App
 * installation token through the fleet rotation machinery. In
 * production this is the tenancy Worker's function; the returned
 * token lives only in the caller's frame for one board's fetch.
 */
export type TokenProvider = (org: string) => Promise<string>;

export interface CollectorDeps {
  db: SqlDb;
  tokens: TokenProvider;
  fetchFn: CollectorFetch;
  /** The board's IngestDO (real DO stub in the Worker; an IngestDO
   * over MemoryIngestStorage in tests). */
  ingestDoFor: (boardId: string) => IngestDO;
  now?: () => string;
}

export type BoardOutcome =
  | { boardId: string; outcome: "landed"; snapshotId: string; emittedEntries: number; pagesFetched: number }
  | { boardId: string; outcome: "unchanged" }
  | { boardId: string; outcome: "deferred"; resetAt: number | null }
  | { boardId: string; outcome: "failed"; error: string };

export interface CollectorRunReport {
  generatedAt: string;
  boards: BoardOutcome[];
  /** Boards whose snapshots landed this run. */
  landedBoardIds: string[];
  failedBoardIds: string[];
}

interface CollectedBoard {
  board: BoardDef;
  snapshotId: string;
  items: ProjectPoolItem[];
  audit: ProjectAudit;
  counts: { reportedTotal: number; emittedEntries: number };
  bodies: Map<string, string>;
  pagesFetched: number;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** One full collector run over the registry. Never throws for a
 * per-board failure (those land in the report + findings); throws
 * only when the run itself cannot start (e.g. a corrupt registry). */
export async function runHostedCollector(deps: CollectorDeps): Promise<CollectorRunReport> {
  const now = deps.now ?? (() => new Date().toISOString());
  const generatedAt = now();
  const boards = loadRegistry(deps.db);
  const outcomes = new Map<string, BoardOutcome>();
  const collected: CollectedBoard[] = [];

  for (const board of boards) {
    const outcome = await deps.ingestDoFor(board.id).runSerialized<BoardOutcome>(async (state) => {
      const runAt = now();
      const fail = (error: unknown, rate: RateBudget): { result: BoardOutcome; next: IngestState } => {
        const message = errorMessage(error);
        recordIngestFinding(deps.db, {
          findingId: `ingest-${board.id}-${runAt}`,
          boardId: board.id,
          kind: "ingest-failure",
          severity: "error",
          summary: `hosted collector ingest failed for board ${board.id}: ${message}`,
          detail: { error: message },
          now: runAt,
        });
        return {
          result: { boardId: board.id, outcome: "failed", error: message },
          next: { ...state, rate, lastOutcome: "failed", lastRunAt: runAt },
        };
      };

      if (budgetExhausted(state.rate, runAt)) {
        recordIngestFinding(deps.db, {
          findingId: `ingest-${board.id}-${runAt}`,
          boardId: board.id,
          kind: "ingest-deferred",
          severity: "warning",
          summary: `hosted collector deferred board ${board.id}: GitHub rate budget exhausted until ${new Date(
            (state.rate.resetAt as number) * 1000,
          ).toISOString()}`,
          detail: { resetAt: state.rate.resetAt },
          now: runAt,
        });
        return {
          result: { boardId: board.id, outcome: "deferred", resetAt: state.rate.resetAt },
          next: { ...state, lastOutcome: "deferred", lastRunAt: runAt },
        };
      }

      let token: string;
      try {
        token = await deps.tokens(board.org);
      } catch (error) {
        return fail(error, state.rate);
      }
      let fetched: Awaited<ReturnType<typeof fetchBoardSnapshot>>;
      try {
        fetched = await fetchBoardSnapshot({
          token,
          fetchFn: deps.fetchFn,
          generatedAt,
          board,
          etag: state.etag,
        });
      } catch (error) {
        return fail(error, state.rate);
      }
      // The token's last use for this board is above; nothing below
      // receives it, and it is never written to any store.

      if (fetched.kind === "unchanged") {
        return {
          result: { boardId: board.id, outcome: "unchanged" },
          next: { ...state, etag: fetched.etag, rate: fetched.rate, lastOutcome: "unchanged", lastRunAt: runAt },
        };
      }
      // The zero-item guard the laptop collector applies to the
      // overall board: an empty export there is a broken read, not
      // a snapshot — fail the board rather than emptying its pool.
      if (!fetched.items.length && board.role === "overall") {
        return fail(new Error("board returned zero resolvable items"), fetched.rate);
      }
      const snapshotId = `snap-${board.id}-${generatedAt}`;
      collected.push({
        board,
        snapshotId,
        items: fetched.items,
        audit: fetched.audit,
        counts: fetched.counts,
        bodies: fetched.bodies,
        pagesFetched: fetched.pagesFetched,
      });
      return {
        result: {
          boardId: board.id,
          outcome: "landed",
          snapshotId,
          emittedEntries: fetched.counts.emittedEntries,
          pagesFetched: fetched.pagesFetched,
        },
        next: {
          etag: fetched.etag,
          lastSnapshotAt: generatedAt,
          lastSnapshotId: snapshotId,
          rate: fetched.rate,
          lastOutcome: "landed",
          lastRunAt: runAt,
        },
      };
    });
    outcomes.set(board.id, outcome);
  }

  // Cross-board ancestry resolves over the merged pool: epic and
  // spawn resolution run once over the union of this run's boards,
  // then each board's rows land with the resolved fields.
  if (collected.length) {
    const merged = resolveSpawnRollups(resolveEpicRefs(collected.flatMap((entry) => entry.items)));
    const resolvedByBoard = new Map<string, ProjectPoolItem[]>();
    for (const item of merged) {
      const list = resolvedByBoard.get(item.boardId as string) ?? [];
      list.push(item);
      resolvedByBoard.set(item.boardId as string, list);
    }
    for (const entry of collected) {
      landSnapshot(deps.db, {
        snapshotId: entry.snapshotId,
        boardId: entry.board.id,
        generatedAt,
        audit: entry.audit,
        counts: entry.counts,
        items: resolvedByBoard.get(entry.board.id) ?? [],
        bodies: entry.bodies,
        now: now(),
      });
    }
  }

  const boardOutcomes = boards.map((board) => outcomes.get(board.id) as BoardOutcome);
  return {
    generatedAt,
    boards: boardOutcomes,
    landedBoardIds: boardOutcomes.filter((o) => o.outcome === "landed").map((o) => o.boardId),
    failedBoardIds: boardOutcomes.filter((o) => o.outcome === "failed").map((o) => o.boardId),
  };
}

/**
 * The Cron Trigger entrypoint shape: a Worker `scheduled` handler
 * calls this with its bound deps. Kept as a factory so the binding
 * lives at the Worker edge and the run itself stays testable.
 */
export function createScheduledHandler(
  deps: CollectorDeps,
): (event: { cron: string; scheduledTime: number }) => Promise<CollectorRunReport> {
  return () => runHostedCollector(deps);
}
