// Tests for opencode-route-proxy's locally-answered paths (#2229).
// Run: node --test bin/opencode-route-proxy.test.mjs
//
// The proxy fronts the standing OpenCode server, which these tests never
// reach: they cover health, the model list, malformed bodies (400),
// empty completions (400), unknown routes (404), and an unreachable
// OpenCode server (502, with the service file pointed at a fixture).
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import fs from "node:fs";
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

const startProxy = async () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "opencode-route-proxy-test-"));
  const serviceFile = path.join(tmp, "service-prod.json");
  fs.writeFileSync(serviceFile, JSON.stringify({ password: "test-only" }));
  const port = await freePort();
  const env = {
    ...process.env,
    HOME: tmp,
    OPENCODE_ROUTE_PROXY_PORT: String(port),
    OPENCODE_ROUTE_PROXY_SERVER: "http://127.0.0.1:1",
    OPENCODE_SERVICE_FILE: serviceFile,
  };
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
