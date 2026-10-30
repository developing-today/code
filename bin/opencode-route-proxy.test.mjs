// Tests for opencode-route-proxy (#2229).
// Run: node --test bin/opencode-route-proxy.test.mjs
//
// Locally-answered paths (health, model list, malformed bodies, unknown
// routes) never reach an upstream. The completion paths run against a
// fake OpenCode server (plain HTTP, the v2 shapes the proxy reads):
// finish:"error" propagation, finish_reason mapping, the request
// deadline aborting a stalled upstream, and client disconnect
// interrupting the upstream session.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import fs from "node:fs";
import { createServer } from "node:http";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const binDir = path.dirname(fileURLToPath(import.meta.url));

const freePort = () =>
  new Promise((resolve, reject) => {
    const srv = net.createServer();
    srv.once("error", reject);
    srv.listen(0, "127.0.0.1", () => {
      const { port } = srv.address();
      srv.close(() => resolve(port));
    });
  });

const startProxy = async (opts = {}) => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "opencode-route-proxy-test-"));
  const serviceFile = path.join(tmp, "service-prod.json");
  fs.writeFileSync(serviceFile, JSON.stringify({ password: "test-only" }));
  const port = await freePort();
  const env = {
    ...process.env,
    HOME: tmp,
    OPENCODE_ROUTE_PROXY_PORT: String(port),
    OPENCODE_ROUTE_PROXY_SERVER: opts.server || "http://127.0.0.1:1",
    OPENCODE_SERVICE_FILE: serviceFile,
  };
  if (opts.timeoutMs)
    env.OPENCODE_ROUTE_PROXY_TIMEOUT_MS = String(opts.timeoutMs);
  const child = spawn(process.execPath, [path.join(binDir, "opencode-route-proxy.mjs")], {
    env,
    stdio: ["ignore", "pipe", "pipe"],
  });
  let log = "";
  child.stdout.on("data", (d) => {
    log += d;
  });
  child.stderr.on("data", (d) => {
    log += d;
  });
  const base = `http://127.0.0.1:${port}`;
  for (let i = 0; i < 50; i++) {
    try {
      const res = await fetch(`${base}/health`);
      if (res.ok) return { child, base };
    } catch {
      /* not up yet */
    }
    await new Promise((r) => setTimeout(r, 100));
  }
  child.kill("SIGKILL");
  throw new Error(`proxy did not start: ${log}`);
};

test("health, models, 404, and bad requests are answered locally", async () => {
  const { child, base } = await startProxy();
  try {
    const health = await fetch(`${base}/health`);
    assert.equal(health.status, 200);
    assert.equal((await health.json()).service, "opencode-route-proxy");

    const models = await fetch(`${base}/v1/models`);
    assert.equal(models.status, 200);
    const ids = (await models.json()).data.map((m) => m.id);
    assert.ok(ids.includes("workhorse"), `models should advertise workhorse, got ${ids}`);

    const missing = await fetch(`${base}/nope`);
    assert.equal(missing.status, 404);

    const badJson = await fetch(`${base}/v1/chat/completions`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: "{not json",
    });
    assert.equal(badJson.status, 400);

    const empty = await fetch(`${base}/v1/chat/completions`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ model: "workhorse", messages: [] }),
    });
    assert.equal(empty.status, 400);
  } finally {
    child.kill("SIGKILL");
  }
});

test("unreachable OpenCode server surfaces as 502, not a hang or crash", async () => {
  const { child, base } = await startProxy();
  try {
    const res = await fetch(`${base}/v1/chat/completions`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        model: "opencode-route/workhorse",
        messages: [{ role: "user", content: "hello" }],
      }),
    });
    assert.equal(res.status, 502);
    const body = await res.json();
    assert.ok(body.error?.message, "502 should carry an error message");
  } finally {
    child.kill("SIGKILL");
  }
});

test("malformed request shapes get a 400 and the proxy survives", async () => {
  const { child, base } = await startProxy();
  try {
    const shapes = [
      "null",
      JSON.stringify({ messages: { role: "user", content: "hi" } }),
      JSON.stringify({ messages: [null] }),
      JSON.stringify(["not", "an", "object"]),
    ];
    for (const body of shapes) {
      const res = await fetch(`${base}/v1/chat/completions`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body,
      });
      assert.equal(res.status, 400, `shape ${body} should be a 400`);
    }
    const health = await fetch(`${base}/health`);
    assert.equal(health.status, 200, "proxy must still be alive");
  } finally {
    child.kill("SIGKILL");
  }
});

// A fake OpenCode server speaking just the v2 shapes the proxy reads.
// `behavior.wait(res)` / `behavior.messages(res)` shape the turn; the
// session lifecycle calls are counted for cleanup assertions.
const startFakeOpenCode = async (behavior) => {
  const calls = { creates: 0, prompts: 0, interrupts: 0, deletes: 0 };
  const json = (res, status, obj) => {
    res.writeHead(status, { "content-type": "application/json" });
    res.end(JSON.stringify(obj));
  };
  const server = createServer((req, res) => {
    const p = new URL(req.url, "http://127.0.0.1").pathname;
    if (p === "/api/session" && req.method === "POST") {
      calls.creates++;
      return json(res, 200, { data: { id: "ses_fake" } });
    }
    if (p === "/api/session/ses_fake/prompt") {
      calls.prompts++;
      return json(res, 200, {});
    }
    if (p === "/api/session/ses_fake/interrupt") {
      calls.interrupts++;
      return json(res, 200, {});
    }
    if (p === "/api/session/ses_fake" && req.method === "DELETE") {
      calls.deletes++;
      return json(res, 200, {});
    }
    if (p === "/api/experimental/session/ses_fake/wait")
      return behavior.wait(res);
    if (p === "/api/session/ses_fake/message")
      return behavior.messages(res);
    res.writeHead(404);
    res.end();
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address();
  return { server, calls, base: `http://127.0.0.1:${port}` };
};

const postCompletion = (base, init = {}) =>
  fetch(`${base}/v1/chat/completions`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      model: "opencode-route/workhorse",
      messages: [{ role: "user", content: "hello" }],
    }),
    ...init,
  });

test("a finish:error turn surfaces as a proxy error with the upstream text", async () => {
  const fake = await startFakeOpenCode({
    wait: (res) => {
      res.writeHead(204);
      res.end();
    },
    // The v2 error shape (aggregator fixture): the failed assistant
    // message carries finish:"error"; the error text rides on the
    // accompanying idle message.
    messages: (res) => {
      res.writeHead(200, { "content-type": "application/json" });
      res.end(
        JSON.stringify([
          {
            id: "m3",
            type: "idle",
            error: { type: "unknown", message: "429 subscription quota exhausted" },
          },
          { id: "m2", type: "assistant", finish: "error", content: [] },
          { id: "m1", type: "user", text: "go" },
        ]),
      );
    },
  });
  const { child, base } = await startProxy({ server: fake.base });
  try {
    const res = await postCompletion(base);
    assert.equal(res.status, 502, "an errored turn must not be a 200 completion");
    const body = await res.json();
    assert.match(body.error?.message || "", /quota exhausted/);
    assert.equal(fake.calls.deletes, 1, "the errored session is cleaned up");
  } finally {
    child.kill("SIGKILL");
    fake.server.close();
  }
});

test("upstream finish values map to finish_reason (length stays length)", async () => {
  const fake = await startFakeOpenCode({
    wait: (res) => {
      res.writeHead(204);
      res.end();
    },
    messages: (res) => {
      res.writeHead(200, { "content-type": "application/json" });
      res.end(
        JSON.stringify([
          {
            id: "m2",
            type: "assistant",
            finish: "length",
            tokens: { input: 3, output: 4 },
            content: [{ type: "text", text: "truncated ans" }],
          },
          { id: "m1", type: "user", text: "go" },
        ]),
      );
    },
  });
  const { child, base } = await startProxy({ server: fake.base });
  try {
    const res = await postCompletion(base);
    assert.equal(res.status, 200);
    const body = await res.json();
    assert.equal(body.choices[0].message.content, "truncated ans");
    assert.equal(body.choices[0].finish_reason, "length");
    assert.equal(fake.calls.deletes, 1);
  } finally {
    child.kill("SIGKILL");
    fake.server.close();
  }
});

test("the request deadline aborts a stalled upstream wait", async () => {
  const fake = await startFakeOpenCode({
    // The wait never answers: without an abort signal on the fetch the
    // proxy would hold this request forever.
    wait: () => {},
    messages: (res) => {
      res.writeHead(200, { "content-type": "application/json" });
      res.end("[]");
    },
  });
  const { child, base } = await startProxy({
    server: fake.base,
    timeoutMs: 300,
  });
  try {
    const started = Date.now();
    const res = await postCompletion(base);
    const elapsed = Date.now() - started;
    assert.equal(res.status, 502);
    assert.ok(
      elapsed < 5000,
      `deadline should cut the request near 300ms, took ${elapsed}ms`,
    );
    assert.ok(
      fake.calls.interrupts >= 1,
      "the stalled session is interrupted on timeout",
    );
  } finally {
    child.kill("SIGKILL");
    fake.server.closeAllConnections();
    fake.server.close();
  }
});

test("client disconnect interrupts and deletes the upstream session", async () => {
  const fake = await startFakeOpenCode({
    // A healthy but slow turn: never settles, no assistant message yet.
    wait: (res) => {
      res.writeHead(200, { "content-type": "application/json" });
      res.end("{}");
    },
    messages: (res) => {
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify([{ id: "m1", type: "user", text: "go" }]));
    },
  });
  const { child, base } = await startProxy({ server: fake.base });
  try {
    const ctl = new AbortController();
    const pending = postCompletion(base, { signal: ctl.signal }).catch(
      () => null,
    );
    await new Promise((r) => setTimeout(r, 300));
    ctl.abort();
    await pending;
    // The proxy should interrupt + delete the orphaned session promptly,
    // not when the upstream turn eventually finishes.
    const deadline = Date.now() + 5000;
    while (
      (fake.calls.interrupts < 1 || fake.calls.deletes < 1) &&
      Date.now() < deadline
    )
      await new Promise((r) => setTimeout(r, 100));
    assert.ok(
      fake.calls.interrupts >= 1,
      "disconnect interrupts the upstream session",
    );
    assert.ok(
      fake.calls.deletes >= 1,
      "disconnect deletes the upstream session",
    );
  } finally {
    child.kill("SIGKILL");
    fake.server.close();
  }
});
