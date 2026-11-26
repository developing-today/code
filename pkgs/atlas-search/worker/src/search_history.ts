/**
 * The search-history family of retained objects (#9140 design record,
 * build slice 3): one `search_history` version per search in a session.
 *
 * A caller that names a `session` on POST /v1/search gets each answered
 * search kept as the next version of that session's object. The payload
 * holds the query, its filters, the result refs returned and the session;
 * the query text is dropped unless the `search-history` class allows it
 * (NG5, Q1 default: it does not).
 *
 * The writer is off unless the Worker has a `RETAINED` D1 binding and the
 * class is enabled there (design §6). Keeping history never changes or
 * fails a search.
 */
import { RetainedObjectStore, type EnvelopeDraft } from "./retained_object";

/** What a caller may name as a session: short, opaque, no spaces. */
const SESSION = /^[A-Za-z0-9._:-]{1,128}$/;

export function validSession(value: unknown): value is string {
  return typeof value === "string" && SESSION.test(value);
}

export interface SearchHistoryPayload {
  session: string;
  /** Present only under a class that allows query text. */
  query?: string;
  filters: { k: number };
  cache: "hit" | "miss";
  result_refs: string[];
}

export function searchSessionObjectId(session: string): string {
  return `search-session:${session}`;
}

/** The refs of the results a search returned: `chunk:<id>` in result order. */
export function resultRefs(result: unknown): string[] {
  if (typeof result !== "object" || result === null) return [];
  const results = (result as { results?: unknown }).results;
  if (!Array.isArray(results)) return [];
  const refs: string[] = [];
  for (const entry of results) {
    if (typeof entry !== "object" || entry === null) continue;
    const id = (entry as { chunk_id?: unknown }).chunk_id;
    if (typeof id === "number" || (typeof id === "string" && id.length > 0)) refs.push(`chunk:${id}`);
  }
  return refs;
}

export interface AnsweredSearch {
  session: string;
  query: string;
  k: number;
  /** The query-cache key the search ran under: its inputs. */
  key: string;
  cache: "hit" | "miss";
  result: unknown;
}

export function searchHistoryDraft(search: AnsweredSearch, capturedAt: Date): EnvelopeDraft<SearchHistoryPayload> {
  return {
    objectId: searchSessionObjectId(search.session),
    family: "search_history",
    class: "search-history",
    capturedAt: capturedAt.toISOString(),
    inputsRef: `query-key:${search.key}`,
    payload: {
      session: search.session,
      query: search.query,
      filters: { k: search.k },
      cache: search.cache,
      result_refs: resultRefs(search.result),
    },
  };
}

export type SearchRecordOutcome = { recorded: true; version: number } | { recorded: false };

/** Append one answered search when the class is enabled. */
export async function recordSearch(
  db: D1Database | undefined,
  search: AnsweredSearch,
  now: () => number = Date.now,
): Promise<SearchRecordOutcome> {
  if (!db) return { recorded: false };
  const store = new RetainedObjectStore(db, now);
  const rule = await store.getClass("search-history");
  if (!rule?.enabled) return { recorded: false };
  const stored = await store.append(searchHistoryDraft(search, new Date(now())));
  return { recorded: true, version: stored.version };
}
