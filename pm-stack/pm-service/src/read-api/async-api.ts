/**
 * The hosted read API over the async store (CF-5c, #14969): the
 * same four routes, the same #9060 §6 shapes, and the same honest
 * rendering as api.ts — the only difference is that every load is
 * awaited from an AsyncSqlDb (the libSQL/Turso HTTP adapter), so
 * the handler can run where the synchronous bun:sqlite surface
 * cannot: in the pm-service Worker (src/worker.ts).
 *
 * Read-only, exactly like api.ts: GETs only, SELECTs only, and no
 * token is read, minted, or persisted on this path (E §2.3) — the
 * adapter's credential stays inside the adapter's closure.
 */
import type { AsyncSqlDb } from "../tenant-store/libsql-http.ts";
import { deriveGenealogy } from "./genealogy.ts";
import {
  loadBoardSummariesAsync,
  loadPoolAsync,
  loadReleaseSnapshotsAsync,
  loadRoadmapRowsAsync,
} from "./async-store.ts";

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

export async function itemJsonAsync(
  deps: AsyncReadApiDeps,
  ref: string,
): Promise<{ status: number; body: unknown }> {
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

export async function roadmapJsonAsync(deps: AsyncReadApiDeps) {
  const now = deps.now ?? (() => new Date().toISOString());
  const pool = await loadPoolAsync(deps.db);
  return { generatedAt: now(), rows: await loadRoadmapRowsAsync(deps.db, pool) };
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

/** The fetch handler the Worker entry mounts. Only the four GET
 * routes exist; anything else is a JSON 404. */
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
    if (req.method === "GET" && path === "/api/roadmap.json") return json(await roadmapJsonAsync(deps));
    if (req.method === "GET" && path === "/api/releases.json") return json(await releasesJsonAsync(deps));
    return json({ error: `not found: ${path}` }, 404);
  };
}
