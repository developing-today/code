/**
 * The agent-events sink (#5690) and the fleet-queue snapshot /
 * watermark stores (#5693).
 *
 * Sink route: when EVENTS_BROKER_URL is configured, events are
 * POSTed to the queue broker's /events-append verb — the sink's
 * public face. Since #6925 the broker has a public route:
 * https://queue-broker.security.cab (Cloudflare Tunnel ingress on
 * the pm-stack tunnel -> the broker's tailnet listener), gated by
 * a Cloudflare Access application that admits only this Worker's
 * service token (CF-Access-Client-Id / CF-Access-Client-Secret),
 * and by the broker's own X-Agent-Token check behind it. The
 * broker authenticates X-Agent-Token only — it does not read
 * Authorization: Bearer — so the token rides that header.
 *
 * When EVENTS_BROKER_URL is unset, the Worker performs the sink
 * write itself as the outage fallback: the exact INSERT INTO
 * events statement that the broker's /events-append verb executes
 * (turso_queue.rs verb_events_append) and that the Edge Function's
 * AGENT_EVENTS_SINK=turso path used. One sink, one table, one row
 * shape (worker_id 'github-webhook', task_id NULL, whole-second Z
 * timestamps) — never a third implementation with its own
 * semantics.
 */
import { Turso, tArg } from "./turso";

export interface SinkEnv {
  TURSO_QUEUE_URL: string;
  TURSO_QUEUE_TOKEN: string;
  EVENTS_BROKER_URL?: string;
  EVENTS_BROKER_TOKEN?: string;
  EVENTS_BROKER_ACCESS_CLIENT_ID?: string;
  EVENTS_BROKER_ACCESS_CLIENT_SECRET?: string;
}

/** Queue-backend timestamp format: whole seconds, Z (matches EF). */
export function queueNow(d = new Date()): string {
  return d.toISOString().replace(/\.\d{3}Z$/, "Z");
}

export async function appendAgentEvent(env: SinkEnv, kind: string, detail: Record<string, unknown>): Promise<void> {
  if (env.EVENTS_BROKER_URL) {
    const res = await fetch(`${env.EVENTS_BROKER_URL.replace(/\/$/, "")}/events-append`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        ...(env.EVENTS_BROKER_TOKEN ? { "X-Agent-Token": env.EVENTS_BROKER_TOKEN } : {}),
        ...(env.EVENTS_BROKER_ACCESS_CLIENT_ID && env.EVENTS_BROKER_ACCESS_CLIENT_SECRET
          ? {
              "CF-Access-Client-Id": env.EVENTS_BROKER_ACCESS_CLIENT_ID,
              "CF-Access-Client-Secret": env.EVENTS_BROKER_ACCESS_CLIENT_SECRET,
            }
          : {}),
      },
      body: JSON.stringify({ worker_id: "github-webhook", kind, detail }),
    });
    if (!res.ok) throw new Error(`events-append failed: HTTP ${res.status}`);
    const out = (await res.json().catch(() => ({}))) as { ok?: boolean };
    if (out && out.ok === false) throw new Error("events-append failed: broker returned not-ok");
    return;
  }
  const q = new Turso(env.TURSO_QUEUE_URL, env.TURSO_QUEUE_TOKEN);
  await q.execute("INSERT INTO events (worker_id, task_id, kind, detail, created_at) VALUES (?, NULL, ?, ?, ?)", [
    tArg("github-webhook"),
    tArg(kind),
    tArg(JSON.stringify(detail)),
    tArg(queueNow()),
  ]);
}

/**
 * Advance agent_repo_snapshots.head_sha for a default-branch push
 * (fleet-queue, the #5693 home for PG agent.repo_snapshots).
 * Mirrors the Edge Function: side-branch pushes never move head;
 * a missing row is inserted minimal (the fetcher fills the rest).
 * An unknown (NULL) default_branch never counts as a match (#6945):
 * the caller passes the push payload's repository.default_branch so
 * the default can be resolved even when the stored row does not know
 * it yet; with no known default, head_sha is never written.
 * Returns true when a snapshot row was written.
 */
export async function advanceSnapshot(
  env: SinkEnv,
  repo: string,
  after: string,
  ref: string,
  nowIso: string,
  payloadDefaultBranch?: string | null,
): Promise<boolean> {
  const q = new Turso(env.TURSO_QUEUE_URL, env.TURSO_QUEUE_TOKEN);
  const existing = await q.queryText("SELECT repo FROM agent_repo_snapshots WHERE repo = ?", [tArg(repo)]);
  const storedDef = existing
    ? await q.queryText("SELECT default_branch FROM agent_repo_snapshots WHERE repo = ?", [tArg(repo)])
    : null;
  // #6945: an unknown (NULL) default_branch is NOT every branch. Resolve
  // the default from the stored row first, then the push payload's
  // repository.default_branch; when neither is known, no push may move
  // head_sha, and a missing row is seeded (minimal insert) without a head.
  const defBranch = storedDef || payloadDefaultBranch || null;
  const zeroSha = /^0+$/.test(after);
  let head: string | null = null;
  if (!zeroSha && defBranch && ref === `refs/heads/${defBranch}`) head = after;
  if (!head && existing) return false;
  if (head) {
    await q.execute(
      `INSERT INTO agent_repo_snapshots (repo, head_sha, fetched_at, fetch_source)
       VALUES (?, ?, ?, 'webhook')
       ON CONFLICT(repo) DO UPDATE SET head_sha = excluded.head_sha,
         fetched_at = excluded.fetched_at, fetch_source = 'webhook'`,
      [tArg(repo), tArg(head), tArg(nowIso)],
    );
  } else {
    await q.execute(
      `INSERT INTO agent_repo_snapshots (repo, fetched_at, fetch_source)
       VALUES (?, ?, 'webhook')
       ON CONFLICT(repo) DO UPDATE SET fetched_at = excluded.fetched_at,
         fetch_source = 'webhook'`,
      [tArg(repo), tArg(nowIso)],
    );
  }
  return true;
}

/** dirty:<repo> watermark into the fleet-queue watermark store. */
export async function stampDirtyWatermark(env: SinkEnv, repo: string, reason: string, nowIso: string): Promise<void> {
  const q = new Turso(env.TURSO_QUEUE_URL, env.TURSO_QUEUE_TOKEN);
  await q.execute(
    `INSERT INTO agent_watermarks (key, value, updated_at) VALUES (?, ?, ?)
     ON CONFLICT(key) DO UPDATE SET value = excluded.value,
       updated_at = excluded.updated_at`,
    [tArg(`dirty:${repo}`), tArg(JSON.stringify({ reason, at: nowIso })), tArg(nowIso)],
  );
}
