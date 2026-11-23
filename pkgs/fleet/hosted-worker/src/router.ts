// Router for fleet-hosted (#12583). One Worker, route modules per
// front door (design §3.1), one shared plumbing path for every
// non-public route, in order:
//   route match (404) -> authenticate (401/403/503) -> caller-tenancy
//   assertion (403 cross-tenant) -> route scope for service tokens
//   (403) -> door handler.
// Public routes are exactly /health and /capabilities — the hosted
// door's identity surface, which callers must be able to read before
// they hold a credential. Every other route requires auth, including
// unknown paths' absence: an unmatched path is a plain 404 and
// advertises nothing about the doors.

import {
  assertCallerTenancy,
  authenticate,
  authorizeRoute,
  type AuthDeps,
  type CallerContext,
} from "./auth.ts";
import { buildCapabilitiesDocument } from "./capabilities.ts";
import { FailClosed } from "./errors.ts";
import { buildHealthDocument } from "./health.ts";
import { routes as ingestionRoutes } from "./routes/ingestion.ts";
import { routes as queryRoutes } from "./routes/query.ts";
import { routes as queueRoutes } from "./routes/queue.ts";
import { routes as stampingRoutes } from "./routes/stamping.ts";
import { routes as syncRoutes } from "./routes/sync.ts";

export interface RouteDef {
  method: string;
  path: string;
  /** Door id for service-token scoping; null for the public meta routes. */
  door: string | null;
  public?: boolean;
  handler: (
    req: Request,
    ctx: CallerContext | null,
    deps: AuthDeps,
  ) => Promise<Response> | Response;
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

const META_ROUTES: RouteDef[] = [
  {
    method: "GET",
    path: "/health",
    door: null,
    public: true,
    handler: (_req, _ctx, deps) => json(buildHealthDocument(deps.config)),
  },
  {
    method: "GET",
    path: "/capabilities",
    door: null,
    public: true,
    handler: (_req, _ctx, deps) => json(buildCapabilitiesDocument(deps.config)),
  },
];

export const ROUTES: RouteDef[] = [
  ...META_ROUTES,
  ...stampingRoutes,
  ...queryRoutes,
  ...queueRoutes,
  ...ingestionRoutes,
  ...syncRoutes,
];

export async function dispatch(req: Request, deps: AuthDeps): Promise<Response> {
  try {
    const { pathname } = new URL(req.url);
    const route = ROUTES.find((r) => r.method === req.method && r.path === pathname);
    if (!route) {
      throw new FailClosed(404, "not_found", "no such route");
    }
    if (route.public) {
      return await route.handler(req, null, deps);
    }
    const ctx = await authenticate(req, deps);
    assertCallerTenancy(req, ctx);
    if (route.door) authorizeRoute(ctx, route.door);
    return await route.handler(req, ctx, deps);
  } catch (e) {
    if (e instanceof FailClosed) {
      return json({ error: e.code, message: e.message }, e.status);
    }
    console.error("fleet-hosted: unhandled error", e);
    return json({ error: "internal", message: "internal error" }, 500);
  }
}
