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
 */
import { createAsyncReadApiHandler } from "./read-api/async-api.ts";
import { libsqlHttpDb } from "./tenant-store/libsql-http.ts";

export interface PmServiceEnv {
  VERSION?: string;
  PM_TENANT_DB_URL?: string;
  PM_TENANT_DB_TOKEN?: string;
}

const json = (body: unknown, status = 200): Response =>
  new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });

export async function handlePmServiceFetch(req: Request, env: PmServiceEnv): Promise<Response> {
  const path = new URL(req.url).pathname;
  if (req.method === "GET" && path === "/health") {
    return json({ ok: true, service: "pm-service", version: env.VERSION ?? "unknown" });
  }
  if (!env.PM_TENANT_DB_URL) {
    return json({ error: "pm-service tenant store is not configured" }, 503);
  }
  const handler = createAsyncReadApiHandler({
    db: libsqlHttpDb({ url: env.PM_TENANT_DB_URL, token: env.PM_TENANT_DB_TOKEN }),
  });
  return handler(req);
}

export default {
  fetch: (req: Request, env: PmServiceEnv) => handlePmServiceFetch(req, env),
};
