/**
 * The hosted read API (component A design §4 parity list; #9060 §6
 * shapes), served from the tenant DB via store.ts:
 *
 * - GET /api/projects.json — `projects[]`: every registry board with
 *   its latest snapshot audit, plus the merged pool as `items`.
 * - GET /api/item/<id>.json — one pool item by ref
 *   ("owner/repo#123", percent-encoded in the path) with its
 *   genealogy: `ancestors[]`, `release`, and the rendered sentence.
 *   An unknown ref is a structured 404 — chain breaks and missing
 *   data render honestly, never fabricated (#9060 §6).
 * - GET /api/roadmap.json — roadmap rows with the genealogy overlay
 *   (`priority` / `afterRefs` / `releaseRef`) on every row.
 * - GET /api/releases.json — milestone close-out snapshots (#9060
 *   §4): stored snapshots where close-out recorded them, retroactive
 *   computed accounting for the rest.
 *
 * Read-only: the handler answers GETs only and the store layer
 * issues SELECTs only. No token is read, minted, or persisted on
 * this path (E §2.3) — the API serves tenant-DB data, nothing else.
 */
import type { SqlDb } from "../tenant-store/migrate.ts";
import { deriveGenealogy } from "./genealogy.ts";
import { loadBoardSummaries, loadPool, loadReleaseSnapshots, loadRoadmapRows } from "./store.ts";

export interface ReadApiDeps {
  db: SqlDb;
  /** Clock seam for generatedAt stamps (tests pin it). */
  now?: () => string;
}

const json = (body: unknown, status = 200): Response =>
  new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });

export function projectsJson(deps: ReadApiDeps) {
  const now = deps.now ?? (() => new Date().toISOString());
  return {
    generatedAt: now(),
    projects: loadBoardSummaries(deps.db),
    items: loadPool(deps.db),
  };
}

export function itemJson(deps: ReadApiDeps, ref: string): { status: number; body: unknown } {
  const pool = loadPool(deps.db);
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

export function roadmapJson(deps: ReadApiDeps) {
  const now = deps.now ?? (() => new Date().toISOString());
  const pool = loadPool(deps.db);
  return { generatedAt: now(), rows: loadRoadmapRows(deps.db, pool) };
}

export function releasesJson(deps: ReadApiDeps) {
  const now = deps.now ?? (() => new Date().toISOString());
  const pool = loadPool(deps.db);
  return {
    generatedAt: now(),
    source: "tenant DB milestone snapshots + pool accounting (#9060 §4)",
    releases: loadReleaseSnapshots(deps.db, pool),
  };
}

const ITEM_PATH = /^\/api\/item\/(.+)\.json$/;

/** The fetch handler the Worker entry mounts. Only the four GET
 * routes exist; anything else is a JSON 404. */
export function createReadApiHandler(deps: ReadApiDeps): (req: Request) => Promise<Response> {
  return async (req) => {
    const path = new URL(req.url).pathname;
    if (req.method === "GET" && path === "/api/projects.json") return json(projectsJson(deps));
    const itemMatch = ITEM_PATH.exec(path);
    if (req.method === "GET" && itemMatch) {
      let ref: string;
      try {
        ref = decodeURIComponent(itemMatch[1] as string);
      } catch {
        return json({ error: `malformed percent-encoding in path segment: ${itemMatch[1]}` }, 400);
      }
      const { status, body } = itemJson(deps, ref);
      return json(body, status);
    }
    if (req.method === "GET" && path === "/api/roadmap.json") return json(roadmapJson(deps));
    if (req.method === "GET" && path === "/api/releases.json") return json(releasesJson(deps));
    return json({ error: `not found: ${path}` }, 404);
  };
}
