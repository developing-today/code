// Ingestion door — the hosted half of dual ingestion (#12590;
// design §7.3, #9060 §16). Two routes, one check (src/ingestion.ts):
//
//   POST /v1/ingestion/github — the GitHub webhook receiver.
//     GitHub deliveries carry no fleet credential, so this route
//     is public in the router's sense and the handler performs
//     the delivery's own authentication: the X-Hub-Signature-256
//     HMAC over the raw body, verified before any parsing
//     (src/ingestion.ts). Unconfigured secret → 503; missing or
//     bad signature → 401; a verified delivery that does not name
//     its item → 400. Events/actions the check has no business
//     with are acknowledged 200 as ignored — GitHub must not
//     retry them. The check then runs over the item as freshly
//     read through the item source (never the payload's copy),
//     so the webhook and the sweeper answer identically for the
//     same item state.
//   POST /v1/ingestion/sweep — the interval sweeper's operator
//     surface, behind the shared plumbing (a service token
//     scoped to this door is the scheduled caller). Body:
//     { "repos": ["owner/name", ...] } — absent, the deployment's
//     configured scope (INGESTION_REPOS) is swept; neither → 503.
//     The Worker's scheduled handler (src/worker.ts) runs the
//     same sweep over the configured scope on the cron trigger.
//   POST /v1/ingestion/backfill — the recorded backfill (#14312):
//     the same sweep over the same scope body, with the run
//     (counts in/out, failures named by item) stored in the
//     backfill run store and returned (201). Re-running over an
//     unchanged store records 0 inserted / 0 refreshed — the
//     check's idempotency, read back from the record.
//   GET /v1/ingestion/backfill/runs — the recorded runs, oldest
//     first.
//   POST /v1/ingestion/divergence — the body-vs-DB divergence
//     report (#14312) over the same scope body: every item in
//     scope compared against the DB, divergences named by class
//     (missing_row / stale_field / orphan_row) with counts. A
//     read: it heals nothing — the backfill is the heal.
//
// This door is also the create path's heal: an item whose DB
// insert failed at create time (metadata_inserted: false, #12585)
// is stored by the first check that sees it.

import type { AuthDeps, CallerContext } from "../auth.ts";
import { backfillDepsFrom, backfillRunStoreFrom, runDivergenceReport, runMetadataBackfill } from "../backfill.ts";
import { FailClosed } from "../errors.ts";
import {
  ingestionDepsFrom,
  parseWebhookDelivery,
  runIngestionCheck,
  runIngestionSweep,
  verifyWebhookSignature,
} from "../ingestion.ts";
import type { RouteDef } from "../router.ts";

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function caller(ctx: CallerContext | null): CallerContext {
  if (!ctx) {
    throw new FailClosed(401, "missing_token", "ingestion routes require an authenticated caller");
  }
  return ctx;
}

async function githubWebhook(req: Request, _ctx: CallerContext | null, deps: AuthDeps): Promise<Response> {
  // The delivery's authentication first (see the header note):
  // the secret's absence is a deployment gap (503), a missing or
  // unverifying signature a refused delivery (401) — both before
  // the unauthenticated body is parsed or any dependency touched.
  const secret = deps.config.webhookSecret;
  if (!secret) {
    throw new FailClosed(
      503,
      "webhook_not_configured",
      "this deployment has no GitHub webhook secret configured; failing closed",
    );
  }
  const rawBody = await req.text();
  await verifyWebhookSignature(secret, rawBody, req.headers.get("x-hub-signature-256"));

  const parsed = parseWebhookDelivery(req.headers.get("x-github-event"), req.headers.get("x-github-delivery"), rawBody);
  if ("ignored" in parsed) {
    return json({ outcome: "ignored", reason: parsed.ignored });
  }
  const { delivery } = parsed;

  const engine = ingestionDepsFrom(deps);
  const item = await engine.items.getItem(delivery.repo, delivery.kind, delivery.number);
  if (!item) {
    throw new FailClosed(
      404,
      "item_not_visible",
      `the item ${delivery.repo}#${delivery.number} is not visible to this deployment's GitHub read path`,
    );
  }
  const result = await runIngestionCheck(engine, item, "webhook", {
    event: delivery.event,
    action: delivery.action,
    delivery: delivery.deliveryId,
  });
  return json(result);
}

/** The repo scope the operator surfaces (sweep, backfill,
 * divergence) walk: the body's optional { "repos": [...] }, or
 * the deployment's configured scope (INGESTION_REPOS); neither
 * → 503. One parsing path for all three surfaces, so a scope
 * means the same thing to each. */
async function scopeFromBody(req: Request, deps: AuthDeps): Promise<string[]> {
  // The body is optional (an empty POST walks the configured
  // scope); when present it must be a JSON object naming repos.
  const text = await req.text();
  let repos: string[] | undefined;
  if (text.trim() !== "") {
    let parsedBody: unknown;
    try {
      parsedBody = JSON.parse(text);
    } catch {
      throw new FailClosed(400, "invalid_body", "request body is not valid JSON");
    }
    if (typeof parsedBody !== "object" || parsedBody === null || Array.isArray(parsedBody)) {
      throw new FailClosed(400, "invalid_body", "request body must be a JSON object");
    }
    const raw = (parsedBody as Record<string, unknown>)["repos"];
    if (raw !== undefined) {
      if (!Array.isArray(raw) || raw.some((r) => typeof r !== "string")) {
        throw new FailClosed(400, "invalid_body", "repos must be a list of owner/name strings");
      }
      repos = raw as string[];
      for (const repo of repos) {
        if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repo)) {
          throw new FailClosed(400, "invalid_body", `repos entry ${repo} is not in owner/name form`);
        }
      }
    }
  }
  const scope = repos ?? deps.config.ingestionRepos;
  if (scope.length === 0) {
    throw new FailClosed(
      503,
      "ingestion_not_configured",
      "no repo scope: the request names no repos and this deployment configures none (INGESTION_REPOS); failing closed",
    );
  }
  return scope;
}

async function sweep(req: Request, ctx: CallerContext | null, deps: AuthDeps): Promise<Response> {
  caller(ctx);
  const scope = await scopeFromBody(req, deps);
  const engine = ingestionDepsFrom(deps);
  return json(await runIngestionSweep(engine, scope));
}

async function backfill(req: Request, ctx: CallerContext | null, deps: AuthDeps): Promise<Response> {
  caller(ctx);
  const scope = await scopeFromBody(req, deps);
  return json(await runMetadataBackfill(backfillDepsFrom(deps), scope), 201);
}

async function backfillRuns(_req: Request, ctx: CallerContext | null, deps: AuthDeps): Promise<Response> {
  caller(ctx);
  return json({ runs: await backfillRunStoreFrom(deps).list() });
}

async function divergence(req: Request, ctx: CallerContext | null, deps: AuthDeps): Promise<Response> {
  caller(ctx);
  const scope = await scopeFromBody(req, deps);
  return json(await runDivergenceReport(ingestionDepsFrom(deps), scope));
}

export const routes: RouteDef[] = [
  {
    method: "POST",
    path: "/v1/ingestion/github",
    door: "ingestion",
    public: true,
    handler: githubWebhook,
  },
  { method: "POST", path: "/v1/ingestion/sweep", door: "ingestion", handler: sweep },
  { method: "POST", path: "/v1/ingestion/backfill", door: "ingestion", handler: backfill },
  { method: "GET", path: "/v1/ingestion/backfill/runs", door: "ingestion", handler: backfillRuns },
  { method: "POST", path: "/v1/ingestion/divergence", door: "ingestion", handler: divergence },
];
