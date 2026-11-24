// Sync door (#12591; design §7.2, the hosted form of #9016's
// sync-tool scope). The higher-level sync tools as hosted
// endpoints, so hosted/agent callers stop composing raw gh calls:
//
//   POST /v1/sync/project — project-reconcile's write set
//     (add-issue / set-status typed ops, the script's own outbox
//     vocabulary) executed server-side over the fleet App
//     rotation. Retires the script's per-call `gh project
//     item-add` / `gh project item-edit` invocations and the
//     action-outbox file shape for hosted callers.
//   POST /v1/sync/queue-github — the recurring scripted flows
//     (program tending, stewardship verdict posting, pool
//     reviews) as one endpoint: a named flow posts one marked
//     note plus optional label edits, inheriting the rotation,
//     the retry-safety rule (marker-checked posts, serial ops),
//     and value-free errors. Retires the raw `gh issue comment` /
//     `gh pr comment` / `gh issue edit` compositions in those
//     flows' job bodies.
//
// The engine (op/flow parsing, already-applied checks, per-op
// outcomes) is src/sync.ts; the GitHub seams are src/github.ts's
// GitHubProjectSurface and GitHubCommentSurface. Both handlers
// refuse 503 before parsing when their surface is unconfigured,
// like every door's missing dependency.

import type { AuthDeps, CallerContext } from "../auth.ts";
import { FailClosed } from "../errors.ts";
import type { RouteDef } from "../router.ts";
import {
  flowSyncDepsFrom,
  parseFlowSyncRequest,
  parseProjectSyncRequest,
  projectSyncDepsFrom,
  runFlowSync,
  runProjectSync,
} from "../sync.ts";

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function caller(ctx: CallerContext | null): CallerContext {
  if (!ctx) {
    throw new FailClosed(401, "missing_token", "sync routes require an authenticated caller");
  }
  return ctx;
}

async function jsonBody(req: Request): Promise<unknown> {
  try {
    return await req.json();
  } catch {
    throw new FailClosed(400, "invalid_body", "request body is not valid JSON");
  }
}

async function projectSync(req: Request, ctx: CallerContext | null, deps: AuthDeps): Promise<Response> {
  const me = caller(ctx);
  const engine = projectSyncDepsFrom(deps);
  const request = parseProjectSyncRequest(await jsonBody(req));
  return json(await runProjectSync(engine, me.bindingId, request));
}

async function queueGitHubSync(req: Request, ctx: CallerContext | null, deps: AuthDeps): Promise<Response> {
  const me = caller(ctx);
  const engine = flowSyncDepsFrom(deps);
  const request = parseFlowSyncRequest(await jsonBody(req));
  return json(await runFlowSync(engine, me.bindingId, request));
}

export const routes: RouteDef[] = [
  { method: "POST", path: "/v1/sync/project", door: "sync", handler: projectSync },
  { method: "POST", path: "/v1/sync/queue-github", door: "sync", handler: queueGitHubSync },
];
