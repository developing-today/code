// Query door — the hosted Metadata query surface (#12587; design
// §7.1, #9060 §13 + §15). The hosted counterpart of #9074's query
// surface: list/filter issues and PRs by Metadata fields, served
// from the metadata DB with GitHub as fallback for fields the DB
// has not yet ingested, body-vs-DB divergence reported with bodies
// winning on content. The engine is src/query.ts (shared with the
// TS CLI verbs); this module is only the HTTP shape:
//
//   GET  /v1/query?repo=owner/name&lane=...&direct_parent=owner/name#1
//   POST /v1/query  { "repo": "owner/name", "lane": "...", ... }
//
// Both take the same filter (src/query.ts's parseQueryFilter) and
// answer the same document: { filter, count, results }, each result
// carrying the full served Metadata, its source, its divergence
// finding, and its stamp verification. The collector consumes this
// surface as a genealogy input through these routes (documented in
// src/query.ts's header and the package README).
//
// Dependencies: the metadata store is required — absent, the door
// fails closed with 503, like every door. The GitHub item source is
// the fallback + divergence read path; absent, the door still
// serves DB rows (body_checked false, no fallback), because a
// DB-only answer is a true answer as far as it goes — the bulk
// direct-Turso path (§3.3) has the same shape.

import type { AuthDeps, CallerContext } from "../auth.ts";
import { FailClosed } from "../errors.ts";
import { parseQueryFilter, runMetadataQuery } from "../query.ts";
import type { RouteDef } from "../router.ts";
import { importStampVerifyKeys } from "../stamp-sig.ts";
import type { MetadataStore } from "../stamping.ts";

function storeFrom(deps: AuthDeps): MetadataStore {
  if (!deps.metadataStore) {
    throw new FailClosed(
      503,
      "metadata_store_not_configured",
      "this deployment has no metadata store configured; failing closed",
    );
  }
  return deps.metadataStore;
}

function caller(ctx: CallerContext | null): CallerContext {
  if (!ctx) {
    throw new FailClosed(401, "missing_token", "query routes require an authenticated caller");
  }
  return ctx;
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

async function jsonBody(req: Request): Promise<Record<string, unknown>> {
  let parsed: unknown;
  try {
    parsed = await req.json();
  } catch {
    throw new FailClosed(400, "invalid_body", "request body is not valid JSON");
  }
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    throw new FailClosed(400, "invalid_body", "request body must be a JSON object");
  }
  return parsed as Record<string, unknown>;
}

/** GET filters arrive as query parameters; a repeated parameter is
 * a malformed filter, not a multi-value one. */
function paramsFilter(req: Request): Record<string, unknown> {
  const { searchParams } = new URL(req.url);
  const raw: Record<string, unknown> = {};
  searchParams.forEach((value, key) => {
    if (key in raw) {
      throw new FailClosed(400, "invalid_query", `filter parameter ${key} is repeated`);
    }
    raw[key] = value;
  });
  return raw;
}

async function query(req: Request, ctx: CallerContext | null, deps: AuthDeps): Promise<Response> {
  const me = caller(ctx);
  const store = storeFrom(deps);
  const raw = req.method === "GET" ? paramsFilter(req) : await jsonBody(req);
  const filter = parseQueryFilter(raw);
  const verifyKeys = deps.config.stamp.verifyKeys ? await importStampVerifyKeys(deps.config.stamp.verifyKeys) : null;
  const response = await runMetadataQuery({ store, githubItems: deps.githubItems, verifyKeys }, me.bindingId, filter);
  return json(response);
}

export const routes: RouteDef[] = [
  { method: "GET", path: "/v1/query", door: "query", handler: query },
  { method: "POST", path: "/v1/query", door: "query", handler: query },
];
