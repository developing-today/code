// pm-dispatch — authenticated intake for OpenCode sessions + queue work.
//
//   POST /v1/sessions  launch an OpenCode session on the laptop NOW,
//                      tracked by a queue task (assigned immediately) and
//                      usually a GitHub issue.
//   POST /v1/tasks     enqueue-only: a repo-task/v1 packet into the
//                      Supabase queue, optionally with a GitHub issue.
//   GET  /health       unauthenticated {ok, version}, aggregator-style.
//
// Everything else requires auth (GitHub Actions OIDC or the static bearer
// secret — see auth.ts). Binds loopback + the laptop tailnet address only
// (DISPATCH_HOSTS); public reachability arrives later via the pm-stack
// Cloudflare tunnel, not from here.
import { APP_INSTALLED_REPOS, REPO_DIRS, REPO_QUEUES, loadConfig, type Config } from "./config.ts";
import { AuthError, assertCallerRepo, authenticate, type CallerIdentity } from "./auth.ts";
import {
  ValidationError,
  parseSessionRequest,
  parseTaskRequest,
  resolveSessionDirectory,
  titleFromPrompt,
  type SessionRequest,
  type TaskRequest,
} from "./validate.ts";
import { buildPacket, canonicalDedupeKey, describeCaller, dispatchDedupeKey, validatePayload } from "./packet.ts";
import { QueueClient, QueueError, type TaskRow } from "./queue.ts";
import { handleLaunchFailure } from "./launch_failure.ts";
import { GitHubClient } from "./github.ts";
import { OpenCodeClient } from "./opencode.ts";

const cfg: Config = loadConfig();
const queue = cfg.supabaseServiceRoleKey ? new QueueClient(cfg.supabaseUrl, cfg.supabaseServiceRoleKey) : null;
const github = new GitHubClient(cfg.githubAppId, cfg.githubAppPrivateKeyPem);
const opencode = new OpenCodeClient(cfg.opencodeUrl, cfg.opencodeAuthFile);

const MAX_BODY_BYTES = 256 * 1024;
const LEASE_SECONDS = 6 * 60 * 60;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body, null, 2) + "\n", {
    status,
    headers: { "content-type": "application/json" },
  });
}

async function readJsonBody(req: Request): Promise<unknown> {
  const text = await req.text();
  if (text.length > MAX_BODY_BYTES) throw new ValidationError("body: too large");
  if (!text.trim()) throw new ValidationError("body: required JSON object");
  try {
    return JSON.parse(text);
  } catch {
    throw new ValidationError("body: not valid JSON");
  }
}

function requireQueue(): QueueClient {
  if (!queue) {
    throw new QueueError("dispatch is not configured with SUPABASE_SERVICE_ROLE_KEY — queue unavailable");
  }
  return queue;
}

// ---------------------------------------------------------------------------
// Issue helpers
// ---------------------------------------------------------------------------

interface IssueRef {
  number: number;
  url: string;
}

/** Create the tracking issue unless the repo has no app installation.
 * Returns the issue or null, plus any warning for the response. */
async function maybeCreateIssue(
  repo: string,
  title: string,
  body: string,
  labels: string[],
  warnings: string[],
): Promise<IssueRef | null> {
  if (!APP_INSTALLED_REPOS.has(repo)) {
    warnings.push(`github app is not installed on ${repo}; issue creation skipped (task is the tracking record)`);
    return null;
  }
  try {
    return await github.createIssue(repo, title, body, labels);
  } catch (e) {
    warnings.push(`issue creation failed: ${e instanceof Error ? e.message : String(e)}`);
    return null;
  }
}

// ---------------------------------------------------------------------------
// POST /v1/sessions
// ---------------------------------------------------------------------------

async function launchSession(
  caller: CallerIdentity,
  req: SessionRequest,
  task: TaskRow,
  issue: IssueRef | null,
  directory: string,
): Promise<string> {
  const session = await opencode.createSession({
    directory,
    title: req.title ?? titleFromPrompt(req.prompt),
    model: req.model,
  });
  const prefix = `[dispatch] Queue task #${task.id} · Issue ${issue ? issue.url : "none"} · Repo ${req.repo}`;
  await opencode.sendPrompt(session.id, `${prefix}\n\n${req.prompt}`);

  // Assign immediately: PATCH the exact row the enqueue created (the
  // claim RPC claims the next task in a queue, not a specific one). The
  // worker identity must exist in agent.workers first (worker_id is an
  // FK), so register it idempotently.
  await requireQueue().ensureWorker();
  const now = new Date();
  await requireQueue().patchTask(task.id, {
    status: "claimed",
    worker_id: "dispatch-opencode",
    claimed_at: now.toISOString(),
    lease_until: new Date(now.getTime() + LEASE_SECONDS * 1000).toISOString(),
    progress: {
      lane: "opencode",
      session_id: session.id,
      issue_number: issue?.number ?? null,
      launched_by: caller,
    },
  });

  // Lane ledger: the same assignment the queue workers record after a
  // claim, so a later completion can record the executed lane. The
  // launch itself is already real at this point, so a ledger failure is
  // noted on the task rather than failing the dispatch.
  try {
    await requireQueue().assignLane(task.id);
  } catch (e) {
    try {
      await requireQueue().patchTask(task.id, {
        last_error: `lane assignment failed (launch unaffected): ${e instanceof Error ? e.message : String(e)}`,
      });
    } catch {
      /* nothing more to do — the claim stands */
    }
  }

  if (issue) {
    // Post-launch annotations are best-effort: the session is already
    // running and the task already claimed, so a GitHub failure here
    // must not reject the launch — callers cancel the task on a
    // rejected launch (#2901), which would orphan a live session.
    // Noted on the task instead, like the lane-ledger failure above.
    try {
      await github.comment(
        req.repo,
        issue.number,
        `Dispatch launch confirmed.\n\n- Queue task: #${task.id} (${REPO_QUEUES[req.repo]}, status: claimed, worker: dispatch-opencode)\n- OpenCode session: ${session.id}\n- Launched by: ${describeCaller(caller)}`,
      );
      await github.addLabels(req.repo, issue.number, ["queue/claimed", "work/assigned"]);
    } catch (e) {
      try {
        await requireQueue().patchTask(task.id, {
          last_error: `launch annotation failed (launch unaffected): ${e instanceof Error ? e.message : String(e)}`,
        });
      } catch {
        /* nothing more to do — the claim stands */
      }
    }
  }
  return session.id;
}

async function handleSessions(req: Request, caller: CallerIdentity): Promise<Response> {
  const body = parseSessionRequest(await readJsonBody(req));
  assertCallerRepo(caller, body.repo, cfg.repoAllowlist);
  const warnings: string[] = [];
  // Directory scoping (issue #2903) runs before ANY side effect: the
  // tracking issue, queue task and session below are all created after
  // this line, so a rejected directory leaves nothing behind.
  const directory = resolveSessionDirectory(caller.type, body.repo, body.directory, REPO_DIRS);
  const title = body.title ?? titleFromPrompt(body.prompt);

  // 1. Tracking issue (default on).
  let issue: IssueRef | null = null;
  if (body.issue) {
    const footer = `\n\n---\nLaunched by pm-dispatch for ${body.repo}. Queue task and OpenCode session ids follow in a comment below.`;
    issue = await maybeCreateIssue(body.repo, title, body.prompt + footer, ["source/dispatch"], warnings);
  }

  // 2. Enqueue the queue task.
  const kind = issue ? "small_fix" : "triage";
  const dedupeKey = issue ? canonicalDedupeKey(kind, body.repo, issue.number)! : dispatchDedupeKey(body.repo);
  const packet = buildPacket(
    {
      repo: body.repo,
      kind,
      title,
      prompt: body.prompt,
      priority: body.priority,
      caller,
      model: body.model,
      directory,
      issue,
    },
    dedupeKey,
  );
  const payloadErrors = validatePayload(packet.payload);
  if (payloadErrors.length) {
    return json(500, { error: `internal packet validation failed: ${payloadErrors.join("; ")}` });
  }
  let task: TaskRow;
  try {
    task = await requireQueue().enqueue(packet, body.repo);
  } catch (e) {
    return json(502, {
      error: e instanceof Error ? e.message : String(e),
      issue_number: issue?.number ?? null,
      ...(warnings.length ? { warnings } : {}),
    });
  }

  // 3-5. Launch + assign. wait=false: answer after the enqueue and finish
  // the launch asynchronously, recording the outcome on the task row.
  if (!body.wait) {
    void (async () => {
      try {
        await launchSession(caller, body, task, issue, directory);
      } catch (e) {
        // Mirror the wait:true failure path exactly (#2901): cancel the
        // task and note the failure on the issue — never leave a queued
        // task behind for a session that did not launch.
        await handleLaunchFailure(requireQueue(), github, body.repo, task.id, issue?.number ?? null, e);
      }
    })();
    return json(202, {
      task_id: task.id,
      issue_number: issue?.number ?? null,
      queue: REPO_QUEUES[body.repo],
      status: "launching",
      ...(warnings.length ? { warnings } : {}),
    });
  }

  let sessionId: string;
  try {
    sessionId = await launchSession(caller, body, task, issue, directory);
  } catch (e) {
    // Never leave a queued/claimed task behind for a session that did
    // not launch: cancel it and note the failure on the issue (the
    // shared handler the wait:false path also uses, #2901).
    const msg = await handleLaunchFailure(requireQueue(), github, body.repo, task.id, issue?.number ?? null, e);
    return json(502, { task_id: task.id, error: msg, ...(warnings.length ? { warnings } : {}) });
  }

  return json(201, {
    task_id: task.id,
    session_id: sessionId,
    issue_number: issue?.number ?? null,
    queue: REPO_QUEUES[body.repo],
    status: "claimed",
    ...(warnings.length ? { warnings } : {}),
  });
}

// ---------------------------------------------------------------------------
// POST /v1/tasks
// ---------------------------------------------------------------------------

async function handleTasks(req: Request, caller: CallerIdentity): Promise<Response> {
  const body: TaskRequest = parseTaskRequest(await readJsonBody(req));
  assertCallerRepo(caller, body.repo, cfg.repoAllowlist);
  const warnings: string[] = [];
  const directory = REPO_DIRS[body.repo]!;

  // `issue_number` takes precedence over `issue`: when it is present, do
  // NOT create an issue — the task attaches to that existing issue in
  // body.repo (canonical dedupe key + agent-note comment + labels below).
  let issue: IssueRef | null = null;
  if (body.issueNumber !== undefined) {
    issue = {
      number: body.issueNumber,
      url: `https://github.com/${body.repo}/issues/${body.issueNumber}`,
    };
    if (!APP_INSTALLED_REPOS.has(body.repo)) {
      warnings.push(`github app is not installed on ${body.repo}; issue attach skipped (task is the tracking record)`);
    }
  } else if (body.issue) {
    issue = await maybeCreateIssue(body.repo, body.title, body.prompt, ["queue/queued", "source/dispatch"], warnings);
  }

  const dedupeKey =
    body.dedupeKey ??
    (issue ? canonicalDedupeKey(body.kind, body.repo, issue.number) : null) ??
    dispatchDedupeKey(body.repo);
  const packet = buildPacket(
    {
      repo: body.repo,
      kind: body.kind,
      title: body.title,
      prompt: body.prompt,
      priority: body.priority,
      caller,
      model: body.model,
      directory,
      issue,
    },
    dedupeKey,
  );
  const payloadErrors = validatePayload(packet.payload);
  if (payloadErrors.length) {
    return json(500, { error: `internal packet validation failed: ${payloadErrors.join("; ")}` });
  }
  let task: TaskRow;
  try {
    task = await requireQueue().enqueue(packet, body.repo);
  } catch (e) {
    return json(502, {
      error: e instanceof Error ? e.message : String(e),
      issue_number: issue?.number ?? null,
      ...(body.issueNumber !== undefined && issue ? { issue_url: issue.url } : {}),
      ...(warnings.length ? { warnings } : {}),
    });
  }

  if (issue) {
    // Attach is skipped where the GitHub App is not installed (warning
    // already recorded above); the create path needs no skip — its issue
    // exists precisely because the app route worked there.
    const attachSkipped = body.issueNumber !== undefined && !APP_INSTALLED_REPOS.has(body.repo);
    if (!attachSkipped) {
      try {
        await github.comment(
          body.repo,
          issue.number,
          `Filed by pm-dispatch (${describeCaller(caller)}).\n\n- Queue task: #${task.id} (${REPO_QUEUES[body.repo]}, kind: ${body.kind}, priority: ${body.priority})\n- Status: queued`,
        );
      } catch (e) {
        warnings.push(`issue agent-note comment failed: ${e instanceof Error ? e.message : String(e)}`);
      }
      if (body.issueNumber !== undefined) {
        try {
          await github.addLabels(body.repo, issue.number, ["queue/queued", "source/dispatch"]);
        } catch (e) {
          warnings.push(`issue attach labels failed: ${e instanceof Error ? e.message : String(e)}`);
        }
      }
    }
  }

  return json(201, {
    task_id: task.id,
    issue_number: issue?.number ?? null,
    ...(body.issueNumber !== undefined && issue ? { issue_url: issue.url } : {}),
    queue: REPO_QUEUES[body.repo],
    ...(warnings.length ? { warnings } : {}),
  });
}

// ---------------------------------------------------------------------------
// Routing
// ---------------------------------------------------------------------------

// Exported for the HTTP-level tests (test/server.test.ts) — the only
// import side effect below is Bun.serve, guarded by import.meta.main.
export async function handler(req: Request): Promise<Response> {
  const url = new URL(req.url);
  const path = url.pathname;
  try {
    if (req.method === "GET" && (path === "/health" || path === "/api/health")) {
      return json(200, { ok: true, status: "ok", version: cfg.buildVersion });
    }
    if (req.method === "POST" && (path === "/v1/sessions" || path === "/v1/tasks")) {
      const caller = await authenticate(req, {
        apiToken: cfg.apiToken,
        oidcAudience: cfg.oidcAudience,
        allowlist: cfg.repoAllowlist,
      });
      return path === "/v1/sessions" ? await handleSessions(req, caller) : await handleTasks(req, caller);
    }
    return json(404, { error: "not found" });
  } catch (e) {
    if (e instanceof AuthError) return json(401, { error: e.message });
    if (e instanceof ValidationError) return json(400, { error: e.message });
    if (e instanceof QueueError) return json(502, { error: e.message });
    return json(500, { error: e instanceof Error ? e.message : String(e) });
  }
}

if (import.meta.main) {
  const servers = cfg.hosts.map((hostname) => Bun.serve({ hostname, port: cfg.port, fetch: handler }));
  for (const server of servers) {
    console.log(`dispatch listening on http://${server.hostname}:${server.port} (version: ${cfg.buildVersion})`);
  }
}
