/**
 * The hosted read API over the async store (CF-5c, #14969): the
 * same #9060 §6 routes and shapes, and the same honest
 * rendering as api.ts — the only difference is that every load is
 * awaited from an AsyncSqlDb (the libSQL/Turso HTTP adapter), so
 * the handler can run where the synchronous bun:sqlite surface
 * cannot: in the pm-service Worker (src/worker.ts). One route is
 * hosted-only by design: /api/search.json (CF-7, #14947), the
 * indexed-projection search read, which has no synchronous twin
 * because the laptop's search is the unbounded read it replaces.
 *
 * Read-only, exactly like api.ts: GETs only, SELECTs only, and no
 * token is read, minted, or persisted on this path (E §2.3) — the
 * adapter's credential stays inside the adapter's closure.
 */
import type { AsyncSqlDb } from "../tenant-store/libsql-http.ts";
import { deriveGenealogy } from "./genealogy.ts";
import { searchItemsAsync } from "./search.ts";
import {
  loadBoardSummariesAsync,
  loadPoolAsync,
  loadReleaseSnapshotsAsync,
  loadRoadmapRowsAsync,
} from "./async-store.ts";
import { freshnessFromSummaries, overviewFromPool, programFromPool, queueFromPool } from "./waves.ts";

export interface AsyncReadApiDeps {
  db: AsyncSqlDb;
  /** Clock seam for generatedAt stamps (tests pin it). */
  now?: () => string;
}

const json = (body: unknown, status = 200): Response =>
  new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });

export async function projectsJsonAsync(deps: AsyncReadApiDeps) {
  const now = deps.now ?? (() => new Date().toISOString());
  const [projects, items] = await Promise.all([loadBoardSummariesAsync(deps.db), loadPoolAsync(deps.db)]);
  return { generatedAt: now(), projects, items };
}

export async function itemJsonAsync(deps: AsyncReadApiDeps, ref: string): Promise<{ status: number; body: unknown }> {
  const pool = await loadPoolAsync(deps.db);
  const item = pool.find((entry) => entry.ref === ref);
  if (!item) return { status: 404, body: { error: `no pool item with ref ${ref}` } };
  const genealogy = deriveGenealogy(item, pool);
  return {
    status: 200,
    body: {
      item,
      ancestors: genealogy.ancestors,
      release: genealogy.release,
      genealogySentence: genealogy.sentence,
    },
  };
}

/** Hosted search (CF-7, #14947): the FTS5-indexed projection read
 * over the pool — bounded and ranked, never a pool scan. `q` is
 * required; a request without it is a caller error, said as one. */
export async function searchJsonAsync(deps: AsyncReadApiDeps, params: URLSearchParams) {
  const now = deps.now ?? (() => new Date().toISOString());
  const q = params.get("q");
  if (q === null || q.trim() === "") return { status: 400, body: { error: "q: a search query is required" } };
  const num = (name: string) => {
    const raw = params.get(name);
    if (raw === null) return null;
    const n = Number(raw);
    return Number.isFinite(n) ? n : null;
  };
  const result = await searchItemsAsync(deps.db, q, { limit: num("limit"), offset: num("offset") });
  return { status: 200, body: { generatedAt: now(), ...result } };
}

export async function roadmapJsonAsync(deps: AsyncReadApiDeps) {
  const now = deps.now ?? (() => new Date().toISOString());
  const pool = await loadPoolAsync(deps.db);
  return { generatedAt: now(), rows: await loadRoadmapRowsAsync(deps.db, pool) };
}

/** CF-5d program view over Epic ancestry already present in the pool. */
export async function programJsonAsync(deps: AsyncReadApiDeps) {
  const now = deps.now ?? (() => new Date().toISOString());
  const [summaries, pool] = await Promise.all([loadBoardSummariesAsync(deps.db), loadPoolAsync(deps.db)]);
  return { generatedAt: now(), freshness: freshnessFromSummaries(summaries), ...programFromPool(pool) };
}

/** CF-5d (#14970) wave, async twin of api.ts's overviewJson. */
export async function overviewJsonAsync(deps: AsyncReadApiDeps) {
  const now = deps.now ?? (() => new Date().toISOString());
  const [summaries, pool] = await Promise.all([loadBoardSummariesAsync(deps.db), loadPoolAsync(deps.db)]);
  return {
    generatedAt: now(),
    freshness: freshnessFromSummaries(summaries),
    ...overviewFromPool(pool, summaries),
    projects: summaries,
  };
}

/** CF-5d (#14970) wave, async twin of api.ts's queueJson. */
export async function queueJsonAsync(deps: AsyncReadApiDeps) {
  const now = deps.now ?? (() => new Date().toISOString());
  const [summaries, pool] = await Promise.all([loadBoardSummariesAsync(deps.db), loadPoolAsync(deps.db)]);
  return { generatedAt: now(), freshness: freshnessFromSummaries(summaries), queue: queueFromPool(pool) };
}

export async function releasesJsonAsync(deps: AsyncReadApiDeps) {
  const now = deps.now ?? (() => new Date().toISOString());
  const pool = await loadPoolAsync(deps.db);
  return {
    generatedAt: now(),
    source: "tenant DB milestone snapshots + pool accounting (#9060 §4)",
    releases: await loadReleaseSnapshotsAsync(deps.db, pool),
  };
}

const ITEM_PATH = /^\/api\/item\/(.+)\.json$/;

/** The fetch handler the Worker entry mounts. Only the read API's
 * GET routes exist (the §6 routes plus the CF-5d overview, queue and
 * program wave routes); anything else is a JSON 404. */
export function createAsyncReadApiHandler(deps: AsyncReadApiDeps): (req: Request) => Promise<Response> {
  return async (req) => {
    const path = new URL(req.url).pathname;
    if (req.method === "GET" && path === "/api/projects.json") return json(await projectsJsonAsync(deps));
    const itemMatch = ITEM_PATH.exec(path);
    if (req.method === "GET" && itemMatch) {
      let ref: string;
      try {
        ref = decodeURIComponent(itemMatch[1] as string);
      } catch {
        return json({ error: `malformed percent-encoding in path segment: ${itemMatch[1]}` }, 400);
      }
      const { status, body } = await itemJsonAsync(deps, ref);
      return json(body, status);
    }
    if (req.method === "GET" && path === "/api/search.json") {
      const { status, body } = await searchJsonAsync(deps, new URL(req.url).searchParams);
      return json(body, status);
    }
    if (req.method === "GET" && path === "/api/roadmap.json") return json(await roadmapJsonAsync(deps));
    if (req.method === "GET" && path === "/api/releases.json") return json(await releasesJsonAsync(deps));
    if (req.method === "GET" && path === "/api/program.json") return json(await programJsonAsync(deps));
    if (req.method === "GET" && path === "/api/overview.json") return json(await overviewJsonAsync(deps));
    if (req.method === "GET" && path === "/api/queue.json") return json(await queueJsonAsync(deps));
    return json({ error: `not found: ${path}` }, 404);
  };
}
