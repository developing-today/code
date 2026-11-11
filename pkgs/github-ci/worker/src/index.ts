/**
 * github-ci Worker (#5693) — the GitHub webhook receiver, re-homed
 * off the Supabase Edge Function of the same name (ACTIVE v8).
 *
 * Gate: HMAC-SHA256 over the raw body, X-Hub-Signature-256 header,
 * verified with Web Crypto against GITHUB_WEBHOOK_SECRET — the same
 * semantics as the Edge Function (which ran verify_jwt=false; the
 * HMAC is the gate).
 *
 * Behavior (parity with the Edge Function, minus Postgres):
 * - workflow_run (any repo): INSERT OR IGNORE into Turso
 *   fleet-platform ci_events ONLY, id unset (dedupe_key is the
 *   identity), v4 ingest policy: completed runs only, conclusion
 *   != skipped, raw payload kept only for failures. Fatal on
 *   failure (500 -> GitHub retries), as the EF's PG insert was.
 * - Fleet repos only:
 *   - push: advance agent_repo_snapshots.head_sha in fleet-queue
 *     (default-branch pushes only), stamp fetch_source='webhook',
 *     append a 'push' agent event via the #5690 sink.
 *   - pull_request / issues: append an agent event via the sink
 *     and set the agent_watermarks key 'dirty:<repo>' in
 *     fleet-queue.
 * - ping: pong. Everything else: 202 ignored.
 */
import { Turso, iArg, tArg } from "./turso";
import { FLEET_REPOS, buildRunRow, skipReason } from "./policy";
import {
  advanceSnapshot,
  appendAgentEvent,
  queueNow,
  stampDirtyWatermark,
  type SinkEnv,
} from "./sink";

export interface Env extends SinkEnv {
  GITHUB_WEBHOOK_SECRET: string;
  TURSO_PLATFORM_URL: string;
  TURSO_PLATFORM_TOKEN: string;
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
    if (req.method === "GET") return new Response("github-ci worker ok", { status: 200 });
    if (req.method !== "POST") return new Response("method not allowed", { status: 405 });

    const raw = await req.text();
    const secret = env.GITHUB_WEBHOOK_SECRET ?? "";
    if (secret) {
      const given = (req.headers.get("x-hub-signature-256") ?? "").replace("sha256=", "");
      const expected = await hmacHex(secret, raw);
      if (!timingSafeEqual(given, expected)) {
        return new Response("bad signature", { status: 401 });
      }
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

    const repo = (payload.repository ?? {}) as Record<string, unknown>;
    const repoName = String(repo.full_name ?? "unknown");
    const isFleet = FLEET_REPOS.has(repoName);
    const nowIso = new Date().toISOString();

    try {
      // ---------------------------------------------------------- push ---
      if (eventType === "push" && isFleet) {
        const after = String(payload.after ?? "");
        const ref = String(payload.ref ?? "");
        const payloadDefaultBranch =
          typeof repo.default_branch === "string" ? repo.default_branch : null;
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
        };
        await appendAgentEvent(env, eventType, detail);
        await stampDirtyWatermark(env, repoName, eventType, nowIso);
        return json({ ok: true, repo: repoName, event: eventType });
      }

      if (eventType !== "workflow_run") {
        return new Response(`ignored event ${eventType}`, { status: 202 });
      }

      // -------------------------------------------------- workflow_run ---
      const { row, runId, action, conclusion } = buildRunRow(
        payload,
        repoName,
        eventType,
        queueNow(),
      );
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
      return json({ ok: true, repo: repoName, run_id: runId, conclusion: row.conclusion });
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      console.error(`github-ci worker failed (${eventType} ${repoName}): ${msg}`);
      return new Response(`receiver failed: ${msg}`, { status: 500 });
    }
  },
};
