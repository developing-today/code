/**
 * Hosted search over the tenant pool's FTS5 index (CF-7, #14947;
 * #14107 M7) — the indexed-projection shape of the PM heavy surfaces.
 *
 * The laptop's /search reads its whole registry and source files
 * (#14060: 33 s, unbounded). This read never does: the query is an
 * FTS5 MATCH against `pm_items_fts` (migration 0002: titles + refs,
 * trigger-kept from `pm_items`), ranked by bm25, and bounded — the
 * caller asks for at most MAX_LIMIT hits and the count comes from
 * the index, not from materialising the pool.
 *
 * Scope, stated plainly: the projection holds titles and refs, not
 * issue bodies (the collector does not emit bodies), so hosted
 * search is title/ref search. Body search stays on the tunnel tier
 * until the collector emits bodies.
 *
 * Read-only like the rest of the read API: SELECTs only.
 */
import type { AsyncSqlDb } from "../tenant-store/libsql-http.ts";

export interface SearchHit {
  ref: string;
  title: string;
  boardId: string;
  state: string;
  status: string | null;
  url: string;
  /** bm25 rank from the index: lower is a better match. */
  rank: number;
}

export interface SearchResult {
  query: string;
  /** Rows the index matched (one item can sit on two boards; hits
   * are de-duplicated by ref, so hits.length can be below total). */
  total: number;
  hits: SearchHit[];
}

export const SEARCH_DEFAULT_LIMIT = 50;
export const SEARCH_MAX_LIMIT = 200;

const SEARCH_SQL = `SELECT i.ref AS ref, i.title AS title, i.board_id AS boardId,
       i.state AS state, i.status AS status, i.url AS url,
       bm25(pm_items_fts) AS rank
FROM pm_items_fts
JOIN pm_items i ON i.rowid = pm_items_fts.rowid
WHERE pm_items_fts MATCH ?
ORDER BY rank ASC, i.ref ASC
LIMIT ? OFFSET ?`;

const SEARCH_COUNT_SQL = `SELECT COUNT(*) AS n FROM pm_items_fts WHERE pm_items_fts MATCH ?`;

/** Turn a user's words into an FTS5 MATCH string, or null when there
 * is nothing searchable in them. Each term becomes a quoted prefix
 * phrase ("crate" matches "crate" and "crates"); terms are ANDed, as
 * a reader typing several words expects. Quoting every term is also
 * the injection defence: no user input reaches the MATCH grammar as
 * syntax — operators, column filters and bare quotes are just words. */
export function ftsMatch(query: string): string | null {
  const terms = query
    .split(/\s+/)
    .map((term) => term.replace(/["*^]/g, "").trim())
    .filter((term) => term.length > 0);
  if (terms.length === 0) return null;
  return terms.map((term) => `"${term}"*`).join(" ");
}

/** Clamp a caller-asked page size into the served bound. */
export function searchLimit(raw: number | null | undefined): number {
  if (raw === null || raw === undefined || !Number.isFinite(raw) || raw < 1) return SEARCH_DEFAULT_LIMIT;
  return Math.min(Math.floor(raw), SEARCH_MAX_LIMIT);
}

export async function searchItemsAsync(
  db: AsyncSqlDb,
  query: string,
  opts: { limit?: number | null; offset?: number | null } = {},
): Promise<SearchResult> {
  const match = ftsMatch(query);
  if (match === null) return { query, total: 0, hits: [] };
  const limit = searchLimit(opts.limit);
  const offset = Math.max(0, Math.floor(opts.offset ?? 0) || 0);
  const [countRows, rows] = await Promise.all([
    db.all<{ n: number }>(SEARCH_COUNT_SQL, [match]),
    db.all<SearchHit>(SEARCH_SQL, [match, limit, offset]),
  ]);
  // The same ref can be pooled on two boards; the better-ranked row
  // (the query orders by rank first) is the hit, the rest are the
  // same item seen from another board.
  const seen = new Set<string>();
  const hits: SearchHit[] = [];
  for (const row of rows) {
    if (seen.has(row.ref)) continue;
    seen.add(row.ref);
    hits.push(row);
  }
  return { query, total: countRows[0]?.n ?? 0, hits };
}
