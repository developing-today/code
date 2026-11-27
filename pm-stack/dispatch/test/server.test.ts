// HTTP-level tests for POST /v1/sessions directory scoping (issue #2903).
// Drives the exported handler with a stubbed-JWKS OIDC token: no network,
// no queue, no GitHub — any non-JWKS fetch throws, so a test that tried
// to create an issue/task/session would fail loudly instead of 400ing.
import { afterAll, beforeAll, describe, expect, spyOn, test } from "bun:test";
import { createSign, generateKeyPairSync } from "node:crypto";
import { OpenCodeClient } from "../src/opencode.ts";

const { publicKey, privateKey } = generateKeyPairSync("rsa", { modulusLength: 2048 });
const jwk = publicKey.export({ format: "jwk" }) as unknown as Record<string, unknown>;
jwk.kid = "test-kid-1";
jwk.alg = "RS256";

// Use the production constant, never a copy: a hard-coded copy of the
// old bogus .json URL is exactly how code#4886 stayed hidden by tests.
import { GITHUB_JWKS_URL as JWKS_URL } from "../src/auth.ts";
const realFetch = globalThis.fetch;

// #10609: install the throw-on-network stub inside this file's own hook and
// restore the captured snapshot in afterAll. At module scope it leaked into
// every later file in the same Bun process: when opencode.test.ts ran after
// this file, its real http://127.0.0.1 requests hit this guard and 4 tests
// failed only when the file order happened to be that way (CI run 43807605419
// vs. a green re-dispatch of the identical head). installFetchStub stays a
// function so no assignment happens at import time.
function installFetchStub() {
  globalThis.fetch = (async (input: unknown) => {
    if (String(input) === JWKS_URL) {
      return new Response(JSON.stringify({ keys: [jwk] }), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    }
    throw new Error(`unexpected network fetch in test: ${String(input)}`);
  }) as typeof fetch;
}

beforeAll(installFetchStub);

afterAll(() => {
  globalThis.fetch = realFetch;
  // The runtime's own fetch is native code; a JS function still installed
  // here is the leaked stub (or a stub another file left for us to capture).
  // Failing in this hook is deliberate: it is the explicit restore assertion
  // for #10609, so removing the restore above cannot pass silently.
  expect(Function.prototype.toString.call(globalThis.fetch)).toContain("[native code]");
});

function b64url(data: Buffer | string): string {
  return Buffer.from(data).toString("base64").replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function signOidc(repository: string): string {
  const now = Math.floor(Date.now() / 1000);
  const header = b64url(JSON.stringify({ alg: "RS256", typ: "JWT", kid: "test-kid-1" }));
  const payload = b64url(
    JSON.stringify({
      iss: "https://token.actions.githubusercontent.com",
      aud: "pm-dispatch",
      exp: now + 600,
      iat: now,
      nbf: now - 60,
      repository,
      actor: "test-actor",
      sub: `repo:${repository}:ref:refs/heads/main`,
    }),
  );
  const signer = createSign("RSA-SHA256");
  signer.update(`${header}.${payload}`);
  return `${header}.${payload}.${b64url(signer.sign(privateKey))}`;
}

// src/server.ts loads config at import; the broker token file fixture keeps the queue unconfigured.
// (#5120), so the suite pins a fixture project (no service-role key: the
// queue stays disabled, as these tests intend).
process.env.QUEUE_BROKER_TOKEN_FILE ??= "/nonexistent-dispatch-test/broker-token";
process.env.DISPATCH_API_TOKEN = "dispatch-wake-test-token";
const { handler, taskStatusView, sessionStatusView, LaunchLog } = await import("../src/server.ts");

function sessionsRequest(token: string, body: Record<string, unknown>): Request {
  return new Request("http://x/v1/sessions", {
    method: "POST",
    headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
    body: JSON.stringify(body),
  });
}

// issue:false keeps these tests hermetic (#4917): session requests
// default issue creation ON, and the GitHub client's gh-CLI fallback is
// a subprocess the fetch stub above cannot intercept — with issue left
// on, these tests file REAL tracking issues in developing-today/code
// (the exact orphan class #4917 tracks; baseline runs of this file
// filed #4937/#4938). Issue-creation and failure-compensation behaviour
// is covered hermetically by launch_failure.test.ts stubs.
const base = {
  repo: "developing-today/code",
  prompt: "fix the pm-dispatch session request minimum spec gate",
  issue: false,
};

describe("POST /v1/sessions directory scoping (issue #2903)", () => {
  test("OIDC + foreign directory is rejected with 400 before any side effect", async () => {
    const token = signOidc("developing-today/code");
    const res = await handler(sessionsRequest(token, { ...base, directory: "/tmp/evil" }));
    expect(res.status).toBe(400);
    const body = (await res.json()) as { error: string };
    expect(body.error).toMatch(/directory/);
  });

  test("OIDC + omitted directory passes validation (fails later at the unconfigured queue)", async () => {
    // No broker token in test env, so a request that clears
    // validation reaches requireQueue() and maps to 502 — proving the
    // directory gate let it through. Any network beyond JWKS throws.
    const token = signOidc("developing-today/code");
    const res = await handler(sessionsRequest(token, base));
    expect(res.status).toBe(502);
  });

  test("OIDC + own clone directory passes validation the same way", async () => {
    const token = signOidc("developing-today/code");
    const res = await handler(sessionsRequest(token, { ...base, directory: "/home/user/git/code" }));
    expect(res.status).toBe(502);
  });
});

describe("POST /v1/agent-wake", () => {
  test("starts an OpenCode session that runs fleet agent next for the addressed agent", async () => {
    const createSession = spyOn(OpenCodeClient.prototype, "createSession").mockResolvedValue({ id: "ses_wake_1" });
    const sendPrompt = spyOn(OpenCodeClient.prototype, "sendPrompt").mockResolvedValue(undefined);
    try {
      const response = await handler(
        new Request("http://x/v1/agent-wake", {
          method: "POST",
          headers: { authorization: "Bearer dispatch-wake-test-token", "content-type": "application/json" },
          body: JSON.stringify({
            event: "message.created",
            address: "agent:agent-next-20-local",
            message_ids: [42],
            kind: "instruction",
            from: ["drew"],
            text: "bridge summary that must not become a task prompt",
          }),
        }),
      );

      expect(response.status).toBe(202);
      expect(await response.json()).toEqual({
        agent: "agent-next-20-local",
        session_id: "ses_wake_1",
        status: "started",
      });
      expect(createSession).toHaveBeenCalledWith({
        directory: "/home/user/git/code",
        title: "Fleet wake: agent-next-20-local",
        model: null,
      });
      expect(sendPrompt).toHaveBeenCalledWith(
        "ses_wake_1",
        expect.stringContaining("fleet agent next --agent agent-next-20-local"),
      );
      expect(sendPrompt.mock.calls[0]?.[1]).not.toContain("bridge summary that must not become a task prompt");
    } finally {
      createSession.mockRestore();
      sendPrompt.mockRestore();
    }
  });

  test("rejects channel and malformed addresses without starting a session", async () => {
    const createSession = spyOn(OpenCodeClient.prototype, "createSession").mockResolvedValue({
      id: "ses_wake_invalid",
    });
    try {
      const response = await handler(
        new Request("http://x/v1/agent-wake", {
          method: "POST",
          headers: { authorization: "Bearer dispatch-wake-test-token", "content-type": "application/json" },
          body: JSON.stringify({ event: "message.created", address: "channel:chatgpt", message_ids: [42] }),
        }),
      );
      expect(response.status).toBe(400);
      expect(createSession).not.toHaveBeenCalled();
    } finally {
      createSession.mockRestore();
    }
  });
});

describe("POST /v1/sessions body cap is measured in UTF-8 bytes (issue #3010)", () => {
  test("multibyte body under the cap in code units but over in bytes is rejected as too large", async () => {
    const token = signOidc("developing-today/code");
    // 100,000 CJK chars = 100,000 UTF-16 code units (under the 262,144 cap)
    // but 300,000 UTF-8 bytes (over it).
    const res = await handler(sessionsRequest(token, { ...base, prompt: "界".repeat(100_000) }));
    expect(res.status).toBe(400);
    const body = (await res.json()) as { error: string };
    expect(body.error).toMatch(/too large/);
  });

  test("multibyte body just under the cap in bytes passes the size gate", async () => {
    const token = signOidc("developing-today/code");
    // 87,000 CJK chars = 261,000 UTF-8 bytes plus JSON overhead: under the
    // 262,144-byte cap, so the request clears the size gate and validation
    // and reaches the unconfigured queue (502), like the small-body tests.
    const res = await handler(sessionsRequest(token, { ...base, prompt: "界".repeat(87_000) }));
    expect(res.status).toBe(502);
  });

  test("ASCII body over the cap is rejected as too large, as before", async () => {
    const token = signOidc("developing-today/code");
    const res = await handler(sessionsRequest(token, { ...base, prompt: "a".repeat(262_144) }));
    expect(res.status).toBe(400);
    const body = (await res.json()) as { error: string };
    expect(body.error).toMatch(/too large/);
  });

  test("ASCII body under the cap passes the size gate, as before", async () => {
    const token = signOidc("developing-today/code");
    const res = await handler(sessionsRequest(token, { ...base, prompt: "a".repeat(260_000) }));
    expect(res.status).toBe(502);
  });
});

describe("POST /v1/sessions body is bounded before buffering (issue #10646)", () => {
  // A stream whose chunks dwarf the cap: the handler must cancel early
  // instead of materialising the whole body. `declaredLength` sets the
  // content-length header on the request itself — rewrapping an existing
  // Request around the same stream makes Bun's cancel() hang, so build the
  // request directly.
  function floodingRequest(
    token: string,
    chunks: number,
    declaredLength?: number,
  ): { request: Request; pulled: () => number } {
    let pulled = 0;
    const stream = new ReadableStream<Uint8Array>({
      pull(controller) {
        pulled += 1;
        controller.enqueue(new Uint8Array(64 * 1024).fill(120)); // 'x', invalid JSON on purpose
        if (pulled >= chunks) controller.close();
      },
    });
    return {
      request: new Request("http://x/v1/sessions", {
        method: "POST",
        headers: {
          authorization: `Bearer ${token}`,
          "content-type": "application/json",
          ...(declaredLength === undefined ? {} : { "content-length": String(declaredLength) }),
        },
        body: stream,
        duplex: "half",
      } as RequestInit),
      pulled: () => pulled,
    };
  }

  test("a multi-MiB stream is rejected without reading it all", async () => {
    const token = signOidc("developing-today/code");
    // 64 chunks of 64 KiB = 4 MiB offered; the 256 KiB cap trips after
    // the 5th chunk, so a bounded reader pulls a handful, not dozens.
    const { request, pulled } = floodingRequest(token, 64);
    const res = await handler(request);
    expect(res.status).toBe(400);
    const body = (await res.json()) as { error: string };
    expect(body.error).toMatch(/too large/);
    expect(pulled()).toBeLessThan(64);
  });

  test("an announced-too-large content-length rejects without pulling a chunk", async () => {
    const token = signOidc("developing-today/code");
    const { request, pulled } = floodingRequest(token, 64, 4 * 1024 * 1024);
    const res = await handler(request);
    expect(res.status).toBe(400);
    const body = (await res.json()) as { error: string };
    expect(body.error).toMatch(/too large/);
    // The handler never takes a reader. Bun's Request constructor may
    // prime at most one chunk into its internal buffer, so 1 is the
    // floor here — what matters is that a 4 MiB stream is not drained.
    expect(pulled()).toBeLessThanOrEqual(1);
  });
});

describe("taskStatusView / sessionStatusView (#5555)", () => {
  test("task view surfaces status/worker/progress and payload repo/kind, never the packet body", () => {
    const view = taskStatusView({
      id: 42,
      queue: "repo-health-code-pilot",
      title: "Fix the thing",
      status: "claimed",
      worker_id: "repo-worker-code",
      lane_id: "code-m",
      attempts: 2,
      max_attempts: 6,
      claimed_at: "2026-11-09T00:00:00Z",
      lease_until: "2026-11-09T01:00:00Z",
      created_at: "2026-11-08T00:00:00Z",
      updated_at: "2026-11-09T00:30:00Z",
      completed_at: null,
      progress: { stage: "executing" },
      result: null,
      last_error: null,
      dedupe_key: "repo-health:v1:fix:developing-today/code:issue:1",
      payload: { repo: "developing-today/code", kind: "small_fix", prompt: "verbatim caller prompt" },
      body: "packet body text",
    } as never);
    expect(view.task_id).toBe(42);
    expect(view.status).toBe("claimed");
    expect(view.worker_id).toBe("repo-worker-code");
    expect(view.progress).toEqual({ stage: "executing" });
    expect(view.repo).toBe("developing-today/code");
    expect(view.kind).toBe("small_fix");
    expect("body" in view).toBe(false);
    expect("payload" in view).toBe(false);
    expect(JSON.stringify(view)).not.toContain("verbatim caller prompt");
  });

  test("task view without a payload reports null repo/kind", () => {
    const view = taskStatusView({ id: 7, queue: "q", status: "queued", dedupe_key: "k" } as never);
    expect(view.repo).toBeNull();
    expect(view.kind).toBeNull();
    expect(view.worker_id).toBeNull();
  });

  test("session view picks id/title and the location directory", () => {
    const view = sessionStatusView({
      id: "ses_1",
      title: "work",
      location: { directory: "/home/user/git/code" },
      time: { created: 1, updated: 2 },
      extra: "not forwarded",
    });
    expect(view.session_id).toBe("ses_1");
    expect(view.directory).toBe("/home/user/git/code");
    expect(view.time).toEqual({ created: 1, updated: 2 });
    expect("extra" in view).toBe(false);
  });

  test("session view of a non-object payload yields null fields, not a throw", () => {
    const view = sessionStatusView("nope");
    expect(view.session_id).toBeNull();
    expect(view.title).toBeNull();
    expect(view.directory).toBeNull();
  });
});

describe("GET /v1/tasks/:id and /v1/sessions/:id read routes (#5555)", () => {
  test("GET /v1/tasks/:id without auth is 401", async () => {
    const res = await handler(new Request("http://x/v1/tasks/42"));
    expect(res.status).toBe(401);
  });

  test("GET /v1/sessions/:id without auth is 401", async () => {
    const res = await handler(new Request("http://x/v1/sessions/ses_1"));
    expect(res.status).toBe(401);
  });

  test("GET /v1/tasks/:id with a non-numeric id is 400, before any queue access", async () => {
    const token = signOidc("developing-today/code");
    const res = await handler(new Request("http://x/v1/tasks/abc", { headers: { authorization: `Bearer ${token}` } }));
    expect(res.status).toBe(400);
  });

  test("GET /v1/sessions/:id with malformed percent-encoding is 400, before any upstream call", async () => {
    const token = signOidc("developing-today/code");
    const res = await handler(
      new Request("http://x/v1/sessions/%ZZ", { headers: { authorization: `Bearer ${token}` } }),
    );
    expect(res.status).toBe(400);
  });

  test("GET /v1/tasks/:id authenticated reaches the queue (502 while unconfigured here)", async () => {
    const token = signOidc("developing-today/code");
    const res = await handler(new Request("http://x/v1/tasks/42", { headers: { authorization: `Bearer ${token}` } }));
    expect(res.status).toBe(502);
    const body = (await res.json()) as { error: string };
    expect(body.error).toContain("queue unavailable");
  });
});

describe("launch-path request logging (#6992)", () => {
  function tasksRequest(token: string, body: Record<string, unknown>): Request {
    return new Request("http://x/v1/tasks", {
      method: "POST",
      headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
      body: JSON.stringify(body),
    });
  }

  test("LaunchLog completion line carries method, path, status, total, legs and ids", () => {
    const spy = spyOn(console, "info").mockImplementation(() => {});
    try {
      const log = new LaunchLog();
      log.leg("issue");
      log.leg("enqueue");
      log.done("POST", "/v1/sessions", 201, 42, "ses_abc");
      expect(spy).toHaveBeenCalledTimes(1);
      const line = String(spy.mock.calls[0]?.[0] ?? "");
      expect(line).toContain("[dispatch-request] POST /v1/sessions status=201");
      expect(line).toMatch(/total=\d+ms/);
      expect(line).toContain("issue=");
      expect(line).toContain("enqueue=");
      expect(line).toContain("task=42");
      expect(line).toContain("session=ses_abc");
    } finally {
      spy.mockRestore();
    }
  });

  test("POST /v1/sessions logs its completion line with the enqueue leg (502 while the queue is unconfigured)", async () => {
    const spy = spyOn(console, "info").mockImplementation(() => {});
    try {
      const res = await handler(sessionsRequest(signOidc("developing-today/code"), base));
      expect(res.status).toBe(502);
      const line = spy.mock.calls
        .map((c) => String(c[0] ?? ""))
        .find((l) => l.includes("[dispatch-request] POST /v1/sessions"));
      expect(line).toBeDefined();
      expect(line).toContain("status=502");
      expect(line).toContain("enqueue=");
    } finally {
      spy.mockRestore();
    }
  });

  test("POST /v1/tasks logs its completion line with the enqueue leg (502 while the queue is unconfigured)", async () => {
    const spy = spyOn(console, "info").mockImplementation(() => {});
    try {
      const res = await handler(
        tasksRequest(signOidc("developing-today/code"), {
          repo: "developing-today/code",
          title: "logging probe",
          prompt: "exercise the tasks handler for the logging test",
        }),
      );
      expect(res.status).toBe(502);
      const line = spy.mock.calls
        .map((c) => String(c[0] ?? ""))
        .find((l) => l.includes("[dispatch-request] POST /v1/tasks"));
      expect(line).toBeDefined();
      expect(line).toContain("status=502");
      expect(line).toContain("enqueue=");
    } finally {
      spy.mockRestore();
    }
  });
});
