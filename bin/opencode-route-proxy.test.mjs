// Tests for opencode-route-proxy (#2518, routing-only rebuild).
// Run: node --test bin/opencode-route-proxy.test.mjs
//
// The proxy resolves routes from a fixture opencode.jsonc and calls
// fake LEAF providers (plain OpenAI-compatible HTTP servers). There is
// no OpenCode server anywhere in this setup: completions must succeed
// purely by routing, and every attempt must land in the fixture
// route_attempt ledger with session_id NULL (the zero-session proof).
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import fs from "node:fs";
import { createServer } from "node:http";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { DatabaseSync } from "node:sqlite";

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

// A fake leaf provider. Records what it received; behavior switchable.
const makeLeaf = (name, expectedKey) => {
  const leaf = {
    name,
    mode: "ok", // ok | rate
    last: null,
    hits: 0,
    server: null,
    port: 0,
  };
  leaf.server = createServer((req, res) => {
    let raw = "";
    req.on("data", (d) => (raw += d));
    req.on("end", () => {
      leaf.hits++;
      let body = {};
      try {
        body = JSON.parse(raw || "{}");
      } catch {}
      leaf.last = { headers: req.headers, body, url: req.url };
      if (req.headers.authorization !== `Bearer ${expectedKey}`) {
        res.writeHead(401, { "content-type": "application/json" });
        return res.end(JSON.stringify({ error: { message: "bad key" } }));
      }
      if (leaf.mode === "rate") {
        res.writeHead(429, {
          "content-type": "application/json",
          "retry-after": "30",
        });
        return res.end(JSON.stringify({ error: { message: "quota" } }));
      }
      if (body.stream) {
        res.writeHead(200, { "content-type": "text/event-stream" });
        const chunk = (delta, usage) =>
          `data: ${JSON.stringify({
            id: "chatcmpl-x",
            object: "chat.completion.chunk",
            created: 1,
            model: body.model,
            choices: [
              {
                index: 0,
                delta,
                finish_reason: usage ? "stop" : null,
              },
            ],
            ...(usage ? { usage } : {}),
          })}\n\n`;
        res.write(chunk({ role: "assistant", content: name }));
        res.write(
          chunk({}, { prompt_tokens: 5, completion_tokens: 2, total_tokens: 7 }),
        );
        return res.end("data: [DONE]\n\n");
      }
      res.writeHead(200, { "content-type": "application/json" });
      res.end(
        JSON.stringify({
          id: "chatcmpl-x",
          object: "chat.completion",
          created: 1,
          model: body.model,
          choices: [
            {
              index: 0,
              finish_reason: "stop",
              message: { role: "assistant", content: name },
            },
          ],
          usage: { prompt_tokens: 5, completion_tokens: 2, total_tokens: 7 },
        }),
      );
    });
  });
  return new Promise((resolve) => {
    leaf.server.listen(0, "127.0.0.1", () => {
      leaf.port = leaf.server.address().port;
      resolve(leaf);
    });
  });
};

const ROUTE_ATTEMPT_SCHEMA = `CREATE TABLE route_attempt (
  id integer PRIMARY KEY AUTOINCREMENT,
  time_started integer NOT NULL, time_ended integer NOT NULL,
  session_id text, assistant_message_id text,
  route_id text NOT NULL, provider_id text NOT NULL, model_id text NOT NULL,
  variant text, outcome text NOT NULL,
  error_tag text, error_code text, error_status integer, error_message text,
  retryable integer, output_started integer NOT NULL,
  failed_over_to text, first_token_ms real, response_ms real,
  tokens_per_second real, tokens_input integer, tokens_output integer,
  tokens_reasoning integer, tokens_cache_read integer, tokens_cache_write integer,
  quota text, tokens_estimated integer, error_body text, hedged integer,
  cost_usd real)`;

const startProxy = async ({ nowMs: pinnedNow } = {}) => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "route-proxy-test-"));
  const leafA = await makeLeaf("LEAF-A", "key-a");
  const leafB = await makeLeaf("LEAF-B", "key-b");

  const dbPath = path.join(tmp, "opencode.db");
  const db = new DatabaseSync(dbPath);
  db.exec("PRAGMA journal_mode = WAL");
  db.exec(
    `CREATE TABLE credential (id integer PRIMARY KEY, integration_id text, label text, value text, active integer);
     ${ROUTE_ATTEMPT_SCHEMA}`,
  );
  db.prepare(
    "INSERT INTO credential (integration_id, label, value, active) VALUES (?,?,?,1)",
  ).run("leafB", "test", JSON.stringify({ key: "key-b", type: "api_key" }));
  db.close();

  const configPath = path.join(tmp, "opencode.jsonc");
  fs.writeFileSync(
    configPath,
    `{
  // Fixture config: comments and trailing commas on purpose.
  "disabled_providers": ["leafOff"],
  "provider": {
    "leafA": {
      "npm": "@ai-sdk/openai-compatible",
      "options": { "baseURL": "http://127.0.0.1:${leafA.port}/v1", "apiKey": "key-a" },
      "models": { "m-a": { "variants": { "high": { "options": { "reasoning_effort": "high" } } } } },
    },
    "leafB": {
      "npm": "@ai-sdk/openai-compatible",
      "options": { "baseURL": "http://127.0.0.1:${leafB.port}/v1" },
      "models": { "m-b": {} },
    },
    "leafOff": {
      "npm": "@ai-sdk/openai-compatible",
      "options": { "baseURL": "http://127.0.0.1:${leafB.port}/v1", "apiKey": "key-b" },
      "models": { "m-b": {} },
    },
  },
  "experimental": {
    "model_route_windows": {
      "peak": { "daysOfWeek": ["mon", "tue", "wed", "thu", "fri"], "start": "01:00", "end": "04:00", "timezone": "UTC" },
    },
    "model_routes": {
      "t-ordered": { "selection": "ordered", "targets": ["leafA/m-a", "leafB/m-b"] },
      "t-variant": { "selection": "ordered", "targets": [ { "model": "leafA/m-a", "variants": { "medium": "high" } } ] },
      "t-nested": { "selection": "ordered", "targets": ["opencode-route/t-ordered"] },
      "t-disabled": { "selection": "ordered", "targets": ["leafOff/m-b", "leafB/m-b"] },
      "t-window": { "selection": "ordered", "targets": [ { "model": "leafA/m-a", "window": "peak" }, "leafB/m-b" ] },
      "t-window-inv": { "selection": "ordered", "targets": [ { "model": "leafA/m-a", "window": { "name": "peak", "invert": true } }, "leafB/m-b" ] },
      "t-window-unknown": { "selection": "ordered", "targets": [ { "model": "leafA/m-a", "window": "no-such-window" }, "leafB/m-b" ] },
    },
  },
}`,
  );

  const port = await freePort();
  const env = {
    ...process.env,
    HOME: tmp,
    OPENCODE_ROUTE_PROXY_PORT: String(port),
    OPENCODE_ROUTE_PROXY_CONFIG: configPath,
    OPENCODE_ROUTE_PROXY_DB: dbPath,
    OPENCODE_ROUTE_PROXY_TIMEOUT_MS: "15000",
    ...(pinnedNow
      ? { OPENCODE_ROUTE_PROXY_NOW_MS: String(pinnedNow) }
      : {}),
  };
  const child = spawn(
    process.execPath,
    [path.join(binDir, "opencode-route-proxy.mjs")],
    { env, stdio: ["ignore", "pipe", "pipe"] },
  );
  let log = "";
  child.stdout.on("data", (d) => (log += d));
  child.stderr.on("data", (d) => (log += d));
  const base = `http://127.0.0.1:${port}`;
  for (let i = 0; i < 50; i++) {
    try {
      const res = await fetch(`${base}/health`);
      if (res.ok) break;
    } catch {}
    await new Promise((r) => setTimeout(r, 100));
    if (i === 49) throw new Error(`proxy did not start: ${log}`);
  }
  const attempts = () => {
    const ro = new DatabaseSync(dbPath, { readOnly: true });
    const rows = ro
      .prepare(
        "SELECT route_id, provider_id, model_id, variant, outcome, error_tag, session_id, failed_over_to, output_started FROM route_attempt ORDER BY id",
      )
      .all();
    ro.close();
    return rows;
  };
  const post = (payload) =>
    fetch(`${base}/v1/chat/completions`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(payload),
    });
  const cleanup = async () => {
    child.kill("SIGKILL");
    leafA.server.close();
    leafB.server.close();
  };
  return { base, post, attempts, leafA, leafB, cleanup };
};

const MSGS = [
  { role: "system", content: "be terse" },
  { role: "user", content: "hello" },
];

test("health and model list are served locally", async () => {
  const p = await startProxy();
  try {
    const health = await (await fetch(`${p.base}/health`)).json();
    assert.equal(health.ok, true);
    const models = await (await fetch(`${p.base}/v1/models`)).json();
    const ids = models.data.map((m) => m.id);
    assert.ok(ids.includes("workhorse"));
    assert.ok(ids.every((id) => !id.startsWith("opencode-route/")));
  } finally {
    await p.cleanup();
  }
});

test("ordered route serves from the first leaf, messages verbatim", async () => {
  const p = await startProxy();
  try {
    const res = await p.post({ model: "opencode-route/t-ordered", messages: MSGS });
    assert.equal(res.status, 200);
    const json = await res.json();
    assert.equal(json.choices[0].message.content, "LEAF-A");
    assert.equal(json.model, "opencode-route/t-ordered");
    assert.deepEqual(p.leafA.last.body.messages, MSGS);
    assert.equal(p.leafA.last.body.model, "m-a");
    const rows = p.attempts();
    assert.equal(rows.length, 1);
    assert.equal(rows[0].outcome, "success");
    assert.equal(rows[0].provider_id, "leafA");
    assert.equal(rows[0].session_id, null);
  } finally {
    await p.cleanup();
  }
});

test("429 on the first leaf fails over; both attempts recorded", async () => {
  const p = await startProxy();
  try {
    p.leafA.mode = "rate";
    const res = await p.post({ model: "opencode-route/t-ordered", messages: MSGS });
    assert.equal(res.status, 200);
    const json = await res.json();
    assert.equal(json.choices[0].message.content, "LEAF-B");
    // leafB has no static key in config: it must be served by the
    // credential-store key, which the leaf verifies.
    assert.equal(p.leafB.last.headers.authorization, "Bearer key-b");
    const rows = p.attempts();
    assert.equal(rows.length, 2);
    assert.equal(rows[0].outcome, "failure");
    assert.equal(rows[0].error_tag, "RateLimit");
    assert.equal(rows[0].failed_over_to, "leafB/m-b");
    assert.equal(rows[0].session_id, null);
    assert.equal(rows[1].outcome, "success");
    assert.equal(rows[1].session_id, null);
  } finally {
    await p.cleanup();
  }
});

test("variant remap reaches the leaf as reasoning_effort", async () => {
  const p = await startProxy();
  try {
    const res = await p.post({
      model: "opencode-route/t-variant#medium",
      messages: MSGS,
    });
    assert.equal(res.status, 200);
    assert.equal(p.leafA.last.body.reasoning_effort, "high");
    const rows = p.attempts();
    assert.equal(rows[0].variant, "high");
  } finally {
    await p.cleanup();
  }
});

test("nested routes resolve and disabled providers are skipped", async () => {
  const p = await startProxy();
  try {
    const nested = await p.post({ model: "opencode-route/t-nested", messages: MSGS });
    assert.equal(nested.status, 200);
    assert.equal((await nested.json()).choices[0].message.content, "LEAF-A");
    const dis = await p.post({ model: "opencode-route/t-disabled", messages: MSGS });
    assert.equal(dis.status, 200);
    assert.equal((await dis.json()).choices[0].message.content, "LEAF-B");
  } finally {
    await p.cleanup();
  }
});

test("streaming relays SSE and records a success attempt", async () => {
  const p = await startProxy();
  try {
    const res = await p.post({
      model: "opencode-route/t-ordered",
      messages: MSGS,
      stream: true,
    });
    assert.equal(res.status, 200);
    const text = await res.text();
    assert.ok(text.includes("LEAF-A"));
    assert.ok(text.includes("[DONE]"));
    // The stream attempt is recorded just after the relay ends; poll.
    let rows = [];
    for (let i = 0; i < 20; i++) {
      try {
        rows = p.attempts();
      } catch {}
      if (rows.length) break;
      await new Promise((r) => setTimeout(r, 100));
    }
    assert.equal(rows.length, 1);
    assert.equal(rows[0].outcome, "success");
    assert.equal(rows[0].output_started, 1);
    assert.equal(rows[0].session_id, null);
  } finally {
    await p.cleanup();
  }
});

test("unknown route and malformed bodies are rejected", async () => {
  const p = await startProxy();
  try {
    const res = await p.post({ model: "opencode-route/nope", messages: MSGS });
    assert.equal(res.status, 502);
    assert.match((await res.json()).error.message, /unknown route/);
    const bad = await p.post({ model: "opencode-route/t-ordered", messages: "x" });
    assert.equal(bad.status, 400);
  } finally {
    await p.cleanup();
  }
});

// Window gates (#3225): the fixture "peak" window is Mon-Fri
// 01:00-04:00 UTC. 2026-11-04 is a Wednesday.
const IN_PEAK = Date.UTC(2026, 10, 4, 2, 30);
const OUT_PEAK = Date.UTC(2026, 10, 4, 12, 0);

test("window-gated target is expanded inside its window", async () => {
  const p = await startProxy({ nowMs: IN_PEAK });
  try {
    const res = await p.post({ model: "opencode-route/t-window", messages: MSGS });
    assert.equal(res.status, 200);
    assert.equal((await res.json()).choices[0].message.content, "LEAF-A");
    const rows = p.attempts();
    assert.equal(rows.length, 1);
    assert.equal(rows[0].provider_id, "leafA");
  } finally {
    await p.cleanup();
  }
});

test("window-gated target is excluded outside its window", async () => {
  const p = await startProxy({ nowMs: OUT_PEAK });
  try {
    const res = await p.post({ model: "opencode-route/t-window", messages: MSGS });
    assert.equal(res.status, 200);
    assert.equal((await res.json()).choices[0].message.content, "LEAF-B");
    const rows = p.attempts();
    assert.equal(rows.length, 1);
    assert.equal(rows[0].provider_id, "leafB");
  } finally {
    await p.cleanup();
  }
});

test("inverted window gate is eligible only outside the window", async () => {
  const inP = await startProxy({ nowMs: IN_PEAK });
  try {
    const res = await inP.post({
      model: "opencode-route/t-window-inv",
      messages: MSGS,
    });
    assert.equal(res.status, 200);
    assert.equal((await res.json()).choices[0].message.content, "LEAF-B");
  } finally {
    await inP.cleanup();
  }
  const outP = await startProxy({ nowMs: OUT_PEAK });
  try {
    const res = await outP.post({
      model: "opencode-route/t-window-inv",
      messages: MSGS,
    });
    assert.equal(res.status, 200);
    assert.equal((await res.json()).choices[0].message.content, "LEAF-A");
  } finally {
    await outP.cleanup();
  }
});

test("a gate naming an unknown window fails closed", async () => {
  const p = await startProxy({ nowMs: IN_PEAK });
  try {
    const res = await p.post({
      model: "opencode-route/t-window-unknown",
      messages: MSGS,
    });
    assert.equal(res.status, 200);
    assert.equal((await res.json()).choices[0].message.content, "LEAF-B");
    const rows = p.attempts();
    assert.equal(rows.length, 1);
    assert.equal(rows[0].provider_id, "leafB");
  } finally {
    await p.cleanup();
  }
});
