/**
 * One board's GraphQL snapshot fetch, with the incremental cursors
 * the IngestDO keeps (component A design §3): the board's stored
 * ETag rides the first page request as If-None-Match, and a 304
 * means "nothing changed" — the run ends there with no page fetches
 * and no tenant-DB writes. GitHub's rate-budget headers are captured
 * on every response so the DO can defer a board whose budget is
 * spent instead of failing into it.
 *
 * The token arrives as a parameter, is used only in the
 * Authorization header of these requests, and is never logged,
 * returned, or stored (E §2.3). Fetch is injectable for tests, in
 * the laptop collector's ProjectFetch style.
 */
import {
  parseProjectAudit,
  parseProjectItems,
  projectPageInfo,
  PROJECT_QUERY,
  type GqlItem,
  type GqlProjectPage,
  type ProjectAudit,
  type ProjectPoolItem,
} from "./pool.ts";

/** Narrow fetch shape; the Workers global fetch satisfies it. */
export type CollectorFetch = (input: string | URL | Request, init?: RequestInit) => Promise<Response>;

/** GitHub rate-budget state for one board, from response headers. */
export interface RateBudget {
  limit: number | null;
  remaining: number | null;
  /** Unix seconds at which the budget window resets, when reported. */
  resetAt: number | null;
}

export type BoardFetchResult =
  | {
      kind: "snapshot";
      items: ProjectPoolItem[];
      audit: ProjectAudit;
      counts: { reportedTotal: number; emittedEntries: number };
      /** Issue bodies by ref, for the Metadata copies (design §3/§6).
       * Bodies are content, parsed for §12 blocks by the store; they
       * are never persisted raw. */
      bodies: Map<string, string>;
      etag: string | null;
      rate: RateBudget;
      pagesFetched: number;
    }
  | { kind: "unchanged"; etag: string | null; rate: RateBudget };

function rateBudgetOf(res: Response): RateBudget {
  const num = (name: string): number | null => {
    const raw = res.headers.get(name);
    if (raw === null) return null;
    const value = Number(raw);
    return Number.isFinite(value) ? value : null;
  };
  return {
    limit: num("x-ratelimit-limit"),
    remaining: num("x-ratelimit-remaining"),
    resetAt: num("x-ratelimit-reset"),
  };
}

const GRAPHQL_URL = "https://api.github.com/graphql";

/**
 * Fetch one board's whole pool (paginated to exhaustion, no page
 * cap — the did-not-advance guard is the loop-safety bound, as on
 * the laptop). Throws on HTTP/GraphQL errors so the caller can
 * fail-soft per board. When `etag` is given and GitHub answers 304,
 * returns { kind: "unchanged" } after exactly one request.
 */
export async function fetchBoardSnapshot(input: {
  token: string;
  fetchFn: CollectorFetch;
  generatedAt: string;
  board: { id: string; nodeId: string; url: string };
  etag?: string | null;
}): Promise<BoardFetchResult> {
  const { token, fetchFn, generatedAt, board } = input;
  const parsedItems: ProjectPoolItem[] = [];
  const auditItems: GqlItem[] = [];
  const bodies = new Map<string, string>();
  const seenItemIds = new Set<string>();
  let firstPage: unknown;
  let cursor: string | null = null;
  let etag: string | null = null;
  let rate: RateBudget = { limit: null, remaining: null, resetAt: null };
  let pagesFetched = 0;

  for (let page = 0; ; page++) {
    const headers: Record<string, string> = {
      Authorization: `Bearer ${token}`,
      "Content-Type": "application/json",
      "User-Agent": "pm-service-hosted-collector",
    };
    // The conditional request is only meaningful for the first page:
    // the ETag identifies the board's whole export as last fetched.
    if (page === 0 && input.etag) headers["If-None-Match"] = input.etag;
    const res = await fetchFn(GRAPHQL_URL, {
      method: "POST",
      headers,
      body: JSON.stringify({ query: PROJECT_QUERY, variables: { id: board.nodeId, cursor } }),
      signal: AbortSignal.timeout(20_000),
    });
    rate = rateBudgetOf(res);
    if (res.status === 304 && page === 0) {
      return { kind: "unchanged", etag: input.etag ?? null, rate };
    }
    if (!res.ok) throw new Error(`github GraphQL HTTP ${res.status}`);
    if (page === 0) etag = res.headers.get("etag");
    const json: unknown = await res.json();
    if ((json as GqlProjectPage).errors) throw new Error("github GraphQL returned errors");
    pagesFetched++;
    if (!firstPage) firstPage = json;
    const pageUrl = (json as GqlProjectPage).data?.node?.url ?? board.url;
    const nodes = (json as GqlProjectPage).data?.node?.items?.nodes ?? [];
    // Overlapping pages (the board mutating mid-export) can deliver
    // the same node twice; deduplicate by item id, first delivery
    // wins — the same tolerance the laptop collector applies.
    const freshNodes = nodes.filter((node): node is GqlItem => {
      if (node === null) return false;
      if (!node.id) return true;
      if (seenItemIds.has(node.id)) return false;
      seenItemIds.add(node.id);
      return true;
    });
    auditItems.push(...freshNodes);
    const freshIds = new Set(freshNodes.map((node) => node.id));
    parsedItems.push(...parseProjectItems(json, pageUrl, board.id).filter((item) => freshIds.has(item.id)));
    for (const node of freshNodes) {
      const content = node.content;
      if (
        content &&
        typeof content.body === "string" &&
        typeof content.number === "number" &&
        content.repository?.nameWithOwner
      ) {
        bodies.set(`${content.repository.nameWithOwner}#${content.number}`, content.body);
      }
    }
    const info = projectPageInfo(json);
    if (!info.hasNextPage) {
      if (auditItems.length < info.totalCount) {
        throw new Error(`github GraphQL project export incomplete (${auditItems.length}/${info.totalCount} items)`);
      }
      const audit = parseProjectAudit(firstPage, auditItems, generatedAt);
      return {
        kind: "snapshot",
        items: parsedItems,
        audit,
        counts: {
          reportedTotal: info.totalCount - (auditItems.length - parsedItems.length),
          emittedEntries: parsedItems.length,
        },
        bodies,
        etag,
        rate,
        pagesFetched,
      };
    }
    if (!info.endCursor || info.endCursor === cursor)
      throw new Error("github GraphQL project pagination did not advance");
    cursor = info.endCursor;
  }
}
