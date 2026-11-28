/**
 * github-ci Worker (#5693) — the GitHub webhook receiver; all writes
 * go to Turso.
 *
 * Gate: HMAC-SHA256 over the raw body, X-Hub-Signature-256 header,
 * verified with Web Crypto against GITHUB_WEBHOOK_SECRET (the HMAC is
 * the gate).
 *
 * Behavior:
 * - workflow_run (any repo): INSERT OR IGNORE into Turso
 *   fleet-platform ci_events ONLY, id unset (dedupe_key is the
 *   identity), v4 ingest policy: completed runs only, conclusion
 *   != skipped, raw payload kept only for failures. Fatal on
 *   failure (500 -> GitHub retries).
 * - Fleet repos only:
 *   - push: advance agent_repo_snapshots.head_sha in fleet-queue
 *     (default-branch pushes only), stamp fetch_source='webhook',
 *     append a 'push' agent event via the #5690 sink.
 *   - pull_request / issues: append an agent event via the sink,
 *     set the agent_watermarks key 'dirty:<repo>' in fleet-queue,
 *     and upsert the #8580 GitHub mirror row (src/mirror.ts).
 *   - issue_comment: append an agent event, stamp the watermark,
 *     and refresh the mirror row's comments count / latest
 *     agent-note pointer.
 *   - sub_issues: append an agent event, stamp the watermark, and
 *     move the child issue's parent projection in the mirror
 *     (#14170 Q7; the projector also records the change in
 *     github_issue_history, fail-open).
 *   - pull_request_review: append an agent event and stamp the
 *     watermark (no mirror row for reviews).
 * - C2 (#9570, slice C2-S1 #15834): beside each raw fleet event the
 *   Worker derives its per-kind queue rows (src/derive.ts) into the
 *   same log and raises the count-leg drain signal (src/drain.ts);
 *   the 5-minute cron below raises the age leg.
 * - ping: pong. Everything else: 202 ignored.
 */
import { readBodyUpTo } from "./body";
import { Turso, configureTursoGateway, iArg, tArg, type D1Gateway } from "./turso";
import { FLEET_REPOS, buildRunRow, skipReason } from "./policy";
import { advanceSnapshot, appendAgentEvent, queueNow, stampDirtyWatermark, type SinkEnv } from "./sink";
import { mirrorCommentEvent, mirrorIssueEvent, mirrorPrEvent, mirrorSubIssueEvent } from "./mirror";
import { deriveQueueRows, type DerivedRow } from "./derive";
import { ageSweep, signalOnAppend } from "./drain";

export interface Env extends SinkEnv {
  GITHUB_WEBHOOK_SECRET: string;
  TURSO_PLATFORM_URL: string;
  TURSO_PLATFORM_TOKEN: string;
  /** Deployed commit, stamped per deploy (`--var VERSION:$SHA`); the
   * edge worker-checks probe (#15793) reads it from GET /version. */
  VERSION?: string;
  /** d1-gateway Gateway entrypoint; d1gw:// URLs go through it. */
  D1GW?: D1Gateway;
}

async function hmacHex(secret: string, body: string): Promise<string> {
  const key = await crypto.subtle.importKey(
    "raw",
    new TextEncoder().encode(secret),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const sig = await crypto.subtle.sign("HMAC", key, new TextEncoder().encode(body));
  return [...new Uint8Array(sig)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

function timingSafeEqual(a: string, b: string): boolean {
  if (a.length !== b.length) return false;
  let out = 0;
  for (let i = 0; i < a.length; i++) out |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return out === 0;
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

export default {
  async fetch(req: Request, env: Env): Promise<Response> {
    configureTursoGateway(env.D1GW);
    // #15793: unauthenticated deployed-commit answer for the edge
    // worker-checks probe (pm-stack/console/edge/src/worker-checks.ts
    // reads the `version` field). Only the commit SHA is exposed.
    if (req.method === "GET" && new URL(req.url).pathname === "/version") {
      return json({ ok: true, service: "github-ci", version: env.VERSION ?? "unknown" });
    }
    if (req.method === "GET") return new Response("github-ci worker ok", { status: 200 });
    if (req.method !== "POST") return new Response("method not allowed", { status: 405 });

    const secret = env.GITHUB_WEBHOOK_SECRET ?? "";
    if (!secret.trim()) {
      console.error("github-ci worker rejected webhook: GITHUB_WEBHOOK_SECRET is not configured");
      return new Response("webhook secret not configured", { status: 503 });
    }

    // #11335: bounded read, so an unauthenticated oversized body is
    // refused before it is buffered; the HMAC below stays the gate.
    const raw = await readBodyUpTo(req);
    if (raw === null) return new Response("payload too large", { status: 413 });
    const given = (req.headers.get("x-hub-signature-256") ?? "").replace("sha256=", "");
    const expected = await hmacHex(secret, raw);
    if (!timingSafeEqual(given, expected)) {
      return new Response("bad signature", { status: 401 });
    }

    let payload: Record<string, unknown>;
    try {
      payload = JSON.parse(raw);
    } catch {
      return new Response("bad json", { status: 400 });
    }
    // #6946: JSON.parse also succeeds for null/array/scalar bodies, and
    // dereferencing payload.repository on those threw out of fetch
    // (outside the receiver-failed envelope below). A webhook payload
    // is an object by contract; anything else is a deliberate 400 for
    // every event type, before any dereference.
    if (typeof payload !== "object" || payload === null || Array.isArray(payload)) {
      return new Response("bad payload", { status: 400 });
    }

    const eventType = req.headers.get("x-github-event") ?? "workflow_run";
    if (eventType === "ping") return new Response("pong", { status: 200 });
    const deliveryId = req.headers.get("x-github-delivery");

    const repo = (payload.repository ?? {}) as Record<string, unknown>;
    const repoName = String(repo.full_name ?? "unknown");
    const isFleet = FLEET_REPOS.has(repoName);
    const nowIso = new Date().toISOString();

    // C2-S1 (#15834): append the derived per-kind queue rows for
    // this delivery beside the raw event, then raise the count-leg
    // drain signal (design D4.1). The signal step is fail-open —
    // the rows are already durable, and a signal error must not
    // turn a stored event into a 500 that GitHub retries.
    const appendDerived = async (rows: DerivedRow[]): Promise<void> => {
      for (const row of rows) {
        await appendAgentEvent(env, row.kind, row.detail);
        try {
          await signalOnAppend(env, row.kind, new Date());
        } catch (e) {
          console.error(
            `github-ci drain count signal failed (${row.kind} ${repoName}): ${e instanceof Error ? e.message : String(e)}`,
          );
        }
      }
    };

    try {
      // ---------------------------------------------------------- push ---
      if (eventType === "push" && isFleet) {
        const after = String(payload.after ?? "");
        const ref = String(payload.ref ?? "");
        const payloadDefaultBranch = typeof repo.default_branch === "string" ? repo.default_branch : null;
        await advanceSnapshot(env, repoName, after, ref, nowIso, payloadDefaultBranch);
        await appendAgentEvent(env, "push", { repo: repoName, after, ref });
        return json({ ok: true, repo: repoName, after });
      }

      // ---------------------------------------- pull_request / issues ---
      if ((eventType === "pull_request" || eventType === "issues") && isFleet) {
        const obj = (payload.pull_request ?? payload.issue ?? {}) as Record<string, unknown>;
        const action = String(payload.action ?? "");
        const detail = {
          repo: repoName,
          action,
          number: Number(obj.number ?? 0) || null,
          title: (obj.title as string) ?? null,
          label: ((payload.label as Record<string, unknown> | undefined)?.name as string) ?? null,
        };
        await appendAgentEvent(env, eventType, detail);
        await stampDirtyWatermark(env, repoName, eventType, nowIso);
        // #8580: the mirror row is the read model; a failed write is
        // fatal like the ci_events insert (500 -> GitHub retries).
        if (eventType === "issues") await mirrorIssueEvent(env, payload, repoName);
        else await mirrorPrEvent(env, payload, repoName);
        await appendDerived(deriveQueueRows(eventType, payload, repoName, deliveryId));
        return json({ ok: true, repo: repoName, event: eventType });
      }

      // ------------------------------------------------ issue_comment ---
      if (eventType === "issue_comment" && isFleet) {
        const issue = (payload.issue ?? {}) as Record<string, unknown>;
        const comment = (payload.comment ?? {}) as Record<string, unknown>;
        const detail = {
          repo: repoName,
          action: String(payload.action ?? ""),
          number: Number(issue.number ?? 0) || null,
          comment_id: Number(comment.id ?? 0) || null,
        };
        await appendAgentEvent(env, "issue_comment", detail);
        await stampDirtyWatermark(env, repoName, "issue_comment", nowIso);
        await mirrorCommentEvent(env, payload, repoName);
        await appendDerived(deriveQueueRows(eventType, payload, repoName, deliveryId));
        return json({ ok: true, repo: repoName, event: eventType });
      }

      // ------------------------------------------ pull_request_review ---
      // C2-S1 (#15834): reviews are a queued-kind source (pr_review,
      // mention) but had no intake at all. Append the raw event and
      // stamp the watermark like the other fleet kinds; there is no
      // mirror row for reviews.
      if (eventType === "pull_request_review" && isFleet) {
        const pr = (payload.pull_request ?? {}) as Record<string, unknown>;
        const review = (payload.review ?? {}) as Record<string, unknown>;
        const detail = {
          repo: repoName,
          action: String(payload.action ?? ""),
          number: Number(pr.number ?? 0) || null,
          review_id: Number(review.id ?? 0) || null,
          state: (review.state as string) ?? null,
        };
        await appendAgentEvent(env, "pull_request_review", detail);
        await stampDirtyWatermark(env, repoName, "pull_request_review", nowIso);
        await appendDerived(deriveQueueRows(eventType, payload, repoName, deliveryId));
        return json({ ok: true, repo: repoName, event: eventType });
      }

      // --------------------------------------------------- sub_issues ---
      if (eventType === "sub_issues" && isFleet) {
        const child = (payload.sub_issue ?? {}) as Record<string, unknown>;
        const parent = (payload.parent_issue ?? {}) as Record<string, unknown>;
        const detail = {
          repo: repoName,
          action: String(payload.action ?? ""),
          number: Number(child.number ?? 0) || null,
          parent: Number(parent.number ?? 0) || null,
        };
        await appendAgentEvent(env, "sub_issues", detail);
        await stampDirtyWatermark(env, repoName, "sub_issues", nowIso);
        await mirrorSubIssueEvent(env, payload, repoName);
        return json({ ok: true, repo: repoName, event: eventType });
      }

      if (eventType !== "workflow_run") {
        return new Response(`ignored event ${eventType}`, { status: 202 });
      }

      // -------------------------------------------------- workflow_run ---
      const { row, runId, action, conclusion } = buildRunRow(payload, repoName, eventType, queueNow());
      const skipped = skipReason(action, conclusion);
      if (skipped) {
        return json({ ok: true, skipped, repo: repoName, run_id: runId });
      }
      const platform = new Turso(env.TURSO_PLATFORM_URL, env.TURSO_PLATFORM_TOKEN);
      await platform.execute(
        `INSERT OR IGNORE INTO ci_events (received_at, source, event_type,
           action, repo, workflow_name, workflow_id, run_id, run_attempt,
           branch, sha, status, conclusion, actor, html_url, ci_started_at,
           ci_updated_at, payload, dedupe_key)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
        [
          tArg(row.received_at),
          tArg(row.source),
          tArg(row.event_type),
          tArg(row.action),
          tArg(row.repo),
          tArg(row.workflow_name),
          iArg(row.workflow_id),
          iArg(row.run_id),
          iArg(row.run_attempt),
          tArg(row.branch),
          tArg(row.sha),
          tArg(row.status),
          tArg(row.conclusion),
          tArg(row.actor),
          tArg(row.html_url),
          tArg(row.ci_started_at),
          tArg(row.ci_updated_at),
          tArg(row.payload),
          tArg(row.dedupe_key),
        ],
      );
      // C2-S1 (#15834): a fleet repo's failed default-branch run
      // derives its ci_failure queue row in the event log (the raw
      // event's home is the ci_events row just written).
      if (isFleet) await appendDerived(deriveQueueRows(eventType, payload, repoName, deliveryId));
      return json({
        ok: true,
        repo: repoName,
        run_id: runId,
        conclusion: row.conclusion,
      });
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      console.error(`github-ci worker failed (${eventType} ${repoName}): ${msg}`);
      return new Response(`receiver failed: ${msg}`, { status: 500 });
    }
  },

  // C2 age cron (#15834, design D4.2): every 5 minutes, raise
  // drain-due for any queued kind whose oldest item has reached its
  // age threshold — the age leg cannot fire on an append, and it
  // runs here so the signal is raised even while the Muse host is
  // down. Registered in doc/fleet-schedule-registry.md (Surface 7).
  async scheduled(_event: ScheduledEvent, env: Env): Promise<void> {
    configureTursoGateway(env.D1GW);
    await ageSweep(env, new Date());
  },
};
