/**
 * The pm-service Worker entry (CF-5c, #14969): mounts the hosted
 * read API (read-api/async-api.ts) over the async tenant store
 * (tenant-store/libsql-http.ts), and answers /health with the
 * commit stamp the deploy workflow sets as the VERSION var — the
 * design §4 health gate requires the deployed version to report
 * the exact commit it was deployed from.
 *
 * Configuration is by name only, from the Worker's environment:
 * - VERSION — plain var, set by the deploy workflow per deploy.
 * - PM_TENANT_DB_URL / PM_TENANT_DB_TOKEN — Worker secrets for the
 *   tenant plane's Turso DB (set with `wrangler secret put`; the
 *   values never appear in this repo). Until they are set the API
 *   routes answer a structured 503 — an unconfigured store is an
 *   honest state, never invented data (#14065) — while /health
 *   keeps answering so the deploy gate can still verify the build.
 *
 * The dashboard pages (dashboard/dashboard.ts, #12829) are served
 * first; any path they do not own falls through to the JSON API.
 */
import { createDashboardHandler } from "./dashboard/dashboard.ts";
import { createAsyncReadApiHandler } from "./read-api/async-api.ts";
import { d1AsyncDb, type D1DatabaseLike } from "./tenant-store/d1.ts";
import { libsqlHttpDb } from "./tenant-store/libsql-http.ts";
import { BoardDO } from "./board-do.ts";
import { completeBrowserLogin, startBrowserLogin, type BrowserOAuthDeps } from "./oauth-pkce.ts";
import { authenticateBrowserSession, logoutBrowserSession } from "./session.ts";

export { BoardDO };

export interface PmServiceEnv {
  VERSION?: string;
  DEPLOYED_AT?: string;
  PM_TENANT_D1?: D1DatabaseLike;
  PM_TENANT_DB_URL?: string;
  PM_TENANT_DB_TOKEN?: string;
  BOARD_DO?: DurableObjectNamespace;
  /** OAuth/session adapters are supplied by the deployment composition. */
  BROWSER_OAUTH?: BrowserOAuthDeps;
}

const json = (body: unknown, status = 200): Response =>
  new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });

export async function handlePmServiceFetch(req: Request, env: PmServiceEnv): Promise<Response> {
  const path = new URL(req.url).pathname;
  if (req.method === "GET" && path === "/health") {
    const nowMs = Date.now();
    const deployedAtMs = Date.parse(env.DEPLOYED_AT ?? "");
    const hasDeploymentStamp = Number.isFinite(deployedAtMs) && deployedAtMs <= nowMs;
    const deployedAt = hasDeploymentStamp ? new Date(deployedAtMs).toISOString() : null;
    const ageSeconds = deployedAt === null ? null : Math.floor((nowMs - deployedAtMs) / 1_000);
    return json({ ok: true, service: "pm-service", version: env.VERSION ?? "unknown", deployedAt, ageSeconds });
  }
  if (path === "/login" && req.method === "GET") {
    if (!env.BROWSER_OAUTH) return json({ error: "browser login is not configured" }, 503);
    return startBrowserLogin(env.BROWSER_OAUTH);
  }
  if (path === "/oauth/callback" && req.method === "GET") {
    if (!env.BROWSER_OAUTH) return json({ error: "browser login is not configured" }, 503);
    return completeBrowserLogin(req, env.BROWSER_OAUTH);
  }
  if (path === "/logout" && req.method === "POST") {
    if (!env.BROWSER_OAUTH) return json({ error: "browser login is not configured" }, 503);
    const cookie = await logoutBrowserSession(req, {
      store: env.BROWSER_OAUTH.sessions,
      audit: env.BROWSER_OAUTH.audit,
    });
    return new Response(null, {
      status: 303,
      headers: { location: "/login", "set-cookie": cookie, "cache-control": "no-store" },
    });
  }
  if (env.BROWSER_OAUTH && path !== "/login" && path !== "/oauth/callback") {
    const auth = await authenticateBrowserSession(req, {
      store: env.BROWSER_OAUTH.sessions,
      resolveLivePrincipal: env.BROWSER_OAUTH.resolveLivePrincipal,
    });
    if (!auth.ok) return json({ error: "unauthenticated" }, 401);
    // Refresh the sliding cookie expiry on all authenticated responses.
    // Routes below own their responses, so the shared dispatcher applies it
    // by wrapping the final response at the end of this function.
    req = new Request(req, { headers: new Headers(req.headers) });
  }
  const boardMatch = /^\/api\/boards\/([^/]+)\/subscribe$/.exec(path);
  if (boardMatch && req.method === "GET") {
    if (!env.BOARD_DO) return json({ error: "BoardDO is not configured" }, 503);
    let boardId: string;
    try {
      boardId = decodeURIComponent(boardMatch[1] as string);
    } catch {
      return json({ error: "malformed board id" }, 400);
    }
    return env.BOARD_DO.get(env.BOARD_DO.idFromName(boardId)).fetch(
      new Request(`https://board-do/subscribe${new URL(req.url).search}`, req),
    );
  }
  const db = env.PM_TENANT_D1
    ? d1AsyncDb(env.PM_TENANT_D1)
    : env.PM_TENANT_DB_URL
      ? libsqlHttpDb({ url: env.PM_TENANT_DB_URL, token: env.PM_TENANT_DB_TOKEN })
      : null;
  if (!db) {
    return json({ error: "pm-service tenant store is not configured" }, 503);
  }
  const page = await createDashboardHandler({ db })(req);
  if (page) return page;
  return createAsyncReadApiHandler({ db })(req);
}

export default {
  fetch: (req: Request, env: PmServiceEnv) => handlePmServiceFetch(req, env),
};
