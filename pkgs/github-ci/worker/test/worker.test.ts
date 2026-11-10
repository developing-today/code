import { createExecutionContext, env, fetchMock, waitOnExecutionContext } from "cloudflare:test";
import { describe, expect, it, beforeAll, afterEach, vi } from "vitest";
import worker from "../src/index";

const SECRET = "test-webhook-secret";
const PLATFORM = "https://fleet-platform-dezren39.aws-us-east-1.turso.io";
const QUEUE = "https://fleet-queue-dezren39.aws-us-east-1.turso.io";

async function sign(body: string, secret = SECRET): Promise<string> {
  const key = await crypto.subtle.importKey(
    "raw",
    new TextEncoder().encode(secret),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const sig = await crypto.subtle.sign("HMAC", key, new TextEncoder().encode(body));
  return "sha256=" + [...new Uint8Array(sig)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

function tursoOk() {
  return JSON.stringify({
    results: [{ type: "success", response: { type: "execute", result: { rows: [] } } }, { type: "success" }],
  });
}

async function post(
  body: string,
  event: string,
  sig?: string,
  webhookSecret: string | null = SECRET,
): Promise<Response> {
  const req = new Request("https://github-ci.test/", {
    method: "POST",
    headers: {
      "x-github-event": event,
      ...(sig ? { "x-hub-signature-256": sig } : {}),
    },
    body,
  });
  const ctx = createExecutionContext();
  const workerEnv = { ...env } as unknown as Record<string, unknown>;
  if (webhookSecret === null) {
    delete workerEnv.GITHUB_WEBHOOK_SECRET;
  } else {
    workerEnv.GITHUB_WEBHOOK_SECRET = webhookSecret;
  }
  const res = await worker.fetch(req, workerEnv as unknown as Parameters<typeof worker.fetch>[1]);
  await waitOnExecutionContext(ctx);
  return res;
}

const runPayload = JSON.stringify({
  action: "completed",
  workflow_run: {
    id: 999,
    run_attempt: 1,
    name: "CI",
    workflow_id: 5,
    head_branch: "main",
    head_sha: "deadbeef",
    status: "completed",
    conclusion: "success",
    actor: { login: "dezren39" },
    html_url: "https://example.invalid/run/999",
    run_started_at: "2026-11-07T10:00:00Z",
    updated_at: "2026-11-07T10:01:00Z",
  },
  repository: { full_name: "developing-today/code" },
});

beforeAll(() => {
  fetchMock.activate();
  fetchMock.disableNetConnect();
});
afterEach(() => {
  vi.restoreAllMocks();
  fetchMock.assertNoPendingInterceptors();
});

describe("github-ci worker", () => {
  it.each([
    ["unset", null],
    ["empty", ""],
  ])("rejects unsigned webhooks when the secret is %s and logs the configuration error", async (_name, secret) => {
    const log = vi.spyOn(console, "error").mockImplementation(() => {});
    const res = await post(runPayload, "workflow_run", undefined, secret);

    expect(res.status).toBe(503);
    expect(await res.text()).toBe("webhook secret not configured");
    expect(log).toHaveBeenCalledWith(
      "github-ci worker rejected webhook: GITHUB_WEBHOOK_SECRET is not configured",
    );
    log.mockRestore();
  });

  it.each([
    ["unset", null],
    ["empty", ""],
  ])("rejects ping when the secret is %s", async (_name, secret) => {
    const res = await post("{}", "ping", undefined, secret);
    expect(res.status).toBe(503);
    expect(await res.text()).toBe("webhook secret not configured");
  });

  it("rejects a signature made with the wrong secret", async () => {
    const res = await post(runPayload, "workflow_run", await sign(runPayload, "wrong-secret"));
    expect(res.status).toBe(401);
  });

  it("rejects a bad signature with 401", async () => {
    const res = await post(runPayload, "workflow_run", "sha256=00");
    expect(res.status).toBe(401);
  });

  it("answers ping", async () => {
    const body = JSON.stringify({ zen: "ok" });
    const res = await post(body, "ping", await sign(body));
    expect(res.status).toBe(200);
    expect(await res.text()).toBe("pong");
  });

  it("stores a completed workflow_run into ci_events via INSERT OR IGNORE, id unset", async () => {
    let sqlSeen = "";
    fetchMock
      .get(PLATFORM)
      .intercept({ path: "/v2/pipeline", method: "POST" })
      .reply(200, (opts) => {
        sqlSeen = String(opts.body);
        return tursoOk();
      });
    const res = await post(runPayload, "workflow_run", await sign(runPayload));
    expect(res.status).toBe(200);
    expect(sqlSeen).toContain("INSERT OR IGNORE INTO ci_events");
    expect(sqlSeen).not.toContain("INSERT OR IGNORE INTO ci_events (id");
    expect(sqlSeen).toContain("github:developing-today/code:999:1:completed");
  });

  it("fails the delivery (500) when Turso answers a results-less 200 for the ci_events insert (#6949)", async () => {
    fetchMock.get(PLATFORM).intercept({ path: "/v2/pipeline", method: "POST" }).reply(200, "{}");
    const res = await post(runPayload, "workflow_run", await sign(runPayload));
    expect(res.status).toBe(500);
    expect(await res.text()).toContain("turso pipeline failed: malformed response");
  });

  it("skips non-completed runs without touching Turso", async () => {
    const p = JSON.stringify({
      action: "requested",
      workflow_run: { id: 1, conclusion: null },
      repository: { full_name: "developing-today/code" },
    });
    const res = await post(p, "workflow_run", await sign(p));
    expect(res.status).toBe(200);
    expect((await res.json()) as { skipped?: string }).toMatchObject({ skipped: "not_completed" });
  });

  it("ignores unknown events with 202", async () => {
    const p = JSON.stringify({ repository: { full_name: "developing-today/code" } });
    const res = await post(p, "star", await sign(p));
    expect(res.status).toBe(202);
  });

  it("push on a fleet repo advances the snapshot and appends a sink event", async () => {
    const seen: string[] = [];
    const queueMock = fetchMock.get(QUEUE);
    // snapshot read (1 pipeline) + upsert (1) + event insert (1)
    for (let i = 0; i < 3; i++) {
      queueMock
        .intercept({ path: "/v2/pipeline", method: "POST" })
        .reply(200, (opts) => {
          seen.push(String(opts.body));
          return tursoOk();
        });
    }
    const p = JSON.stringify({
      ref: "refs/heads/main",
      after: "cafe1234",
      repository: { full_name: "developing-today/code" },
    });
    const res = await post(p, "push", await sign(p));
    expect(res.status).toBe(200);
    const all = seen.join("\n");
    expect(all).toContain("agent_repo_snapshots");
    expect(all).toContain("INSERT INTO events");
    expect(all).toContain("github-webhook");
  });
});
