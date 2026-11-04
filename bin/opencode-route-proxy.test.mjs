// Tests for opencode-route-proxy (#2518, routing-only rebuild).
// Run: node --test bin/opencode-route-proxy.test.mjs
//
// The proxy resolves routes from a fixture opencode.jsonc and calls
// fake LEAF providers (plain OpenAI-compatible HTTP servers). There is
// no OpenCode server anywhere in this setup: completions must succeed
// purely by routing, and every attempt must land in the fixture
// route_attempt ledger with session_id NULL (the zero-session proof).
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
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
      if (leaf.mode === "slow") return;
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

const startProxy = async (arg = {}) => {
  const timeout = typeof arg === "string" ? arg : "15000";
  const pinnedNow = arg && typeof arg === "object" ? arg.nowMs : undefined;
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
  // leafNoCred's only credential row is INACTIVE (active = 0), so its
  // credential lookup comes back empty (#3228 fixture).
  db.prepare(
    "INSERT INTO credential (integration_id, label, value, active) VALUES (?,?,?,0)",
  ).run("leafNoCred", "test-inactive", JSON.stringify({ key: "key-a", type: "api_key" }));
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
      "models": { "m-a": { "variants": { "high": { "options": { "reasoning_effort": "high" } } } }, "m-g": {} },
    },
    "leafB": {
      "npm": "@ai-sdk/openai-compatible",
      "options": { "baseURL": "http://127.0.0.1:${leafB.port}/v1" },
      "models": { "m-b": {}, "m-g": {} },
    },
    "openai": {
      "models": { "m-gpt": {} },
    },
    "leafNoCred": {
      "npm": "@ai-sdk/openai-compatible",
      "options": { "baseURL": "http://127.0.0.1:${leafA.port}/v1" },
      "models": { "m-nc": {}, "m-b": {} },
    },
    "leafOff": {
      "npm": "@ai-sdk/openai-compatible",
      "options": { "baseURL": "http://127.0.0.1:${leafB.port}/v1", "apiKey": "key-b" },
      "models": { "m-b": {} },
    },
  },
  "experimental": {
    "provider_routes": {
      "grp-ordered": { "providers": ["leafB", "leafA"], "selection": "ordered" },
      "grp-rr": { "providers": ["leafA", "leafB"], "selection": "round-robin" },
      "grp-weighted": { "providers": ["leafA", "leafB"], "selection": "weighted", "weights": { "leafA": 0, "leafB": 1 } },
    },
    "model_route_windows": {
      "peak": { "daysOfWeek": ["mon", "tue", "wed", "thu", "fri"], "start": "01:00", "end": "04:00", "timezone": "UTC" },
      "overnight": { "daysOfWeek": ["fri"], "start": "22:00", "end": "02:00", "timezone": "UTC" },
      "bad-start-2400": { "start": "24:00", "end": "12:00", "timezone": "UTC" },
      "bad-end-2460": { "start": "09:00", "end": "24:60", "timezone": "UTC" },
      "bad-wrap-99": { "start": "99:99", "end": "99:98", "timezone": "UTC" },
      "bad-start-2500": { "start": "25:00", "end": "12:00", "timezone": "UTC" },
      "bad-end-2500": { "start": "09:00", "end": "25:00", "timezone": "UTC" },
      "edge-early": { "start": "00:00", "end": "00:02", "timezone": "UTC" },
      "edge-late": { "start": "23:58", "end": "23:59", "timezone": "UTC" },
    },
    "model_routes": {
      "t-ordered": { "selection": "ordered", "targets": ["leafA/m-a", "leafB/m-b"] },
      "t-variant": { "selection": "ordered", "targets": [ { "model": "leafA/m-a", "variants": { "medium": "high" } } ] },
      "t-nested": { "selection": "ordered", "targets": ["opencode-route/t-ordered"] },
      "t-disabled": { "selection": "ordered", "targets": ["leafOff/m-b", "leafB/m-b"] },
      "t-nocred-first": { "selection": "ordered", "targets": ["leafNoCred/m-nc", "leafB/m-b"] },
      "t-nocred-only": { "selection": "ordered", "targets": ["leafNoCred/m-nc"] },
      "t-gate-nocred": { "selection": "ordered", "targets": ["leafA/m-a", "leafNoCred/m-b"] },
      "t-window": { "selection": "ordered", "targets": [ { "model": "leafA/m-a", "window": "peak" }, "leafB/m-b" ] },
      "t-window-inv": { "selection": "ordered", "targets": [ { "model": "leafA/m-a", "window": { "name": "peak", "invert": true } }, "leafB/m-b" ] },
      "t-window-unknown": { "selection": "ordered", "targets": [ { "model": "leafA/m-a", "window": "no-such-window" }, "leafB/m-b" ] },
      "t-window-overnight": { "selection": "ordered", "targets": [ { "model": "leafA/m-a", "window": "overnight" }, "leafB/m-b" ] },
      "t-window-bad-start": { "selection": "ordered", "targets": [ { "model": "leafA/m-a", "window": "bad-start-2400" }, "leafB/m-b" ] },
      "t-window-bad-end": { "selection": "ordered", "targets": [ { "model": "leafA/m-a", "window": "bad-end-2460" }, "leafB/m-b" ] },
      "t-window-bad-wrap": { "selection": "ordered", "targets": [ { "model": "leafA/m-a", "window": "bad-wrap-99" }, "leafB/m-b" ] },
      "t-window-bad-start2500": { "selection": "ordered", "targets": [ { "model": "leafA/m-a", "window": "bad-start-2500" }, "leafB/m-b" ] },
      "t-window-bad-end2500": { "selection": "ordered", "targets": [ { "model": "leafA/m-a", "window": "bad-end-2500" }, "leafB/m-b" ] },
      "t-window-edge-early": { "selection": "ordered", "targets": [ { "model": "leafA/m-a", "window": "edge-early" }, "leafB/m-b" ] },
      "t-window-edge-late": { "selection": "ordered", "targets": [ { "model": "leafA/m-a", "window": "edge-late" }, "leafB/m-b" ] },
      "t-group": { "selection": "ordered", "targets": ["grp-ordered/m-g"] },
      "t-group-rr": { "selection": "ordered", "targets": ["grp-rr/m-g"] },
      "t-group-weighted": { "selection": "ordered", "targets": ["grp-weighted/m-g"] },
      "t-group-skip": { "selection": "ordered", "targets": ["grp-ordered/m-a"] },
      "t-openai-default": { "selection": "ordered", "targets": ["openai/m-gpt"] },
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
    OPENCODE_ROUTE_PROXY_TIMEOUT_MS: timeout,
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
  return { base, post, attempts, leafA, leafB, cleanup, dbPath, log: () => log };
};

const MSGS = [
  { role: "system", content: "be terse" },
  { role: "user", content: "hello" },
];

test("invalid timeout exits at startup with a useful diagnostic", () => {
  for (const raw of ["0", "-1", "1.5", "1e3", "0x10", " 100", "2147483648", "3000000000", "abc", "nope"]) {
    const result = spawnSync(process.execPath, [path.join(binDir, "opencode-route-proxy.mjs")], {
      env: { ...process.env, OPENCODE_ROUTE_PROXY_PORT: "18750", OPENCODE_ROUTE_PROXY_TIMEOUT_MS: raw },
      encoding: "utf8", timeout: 5000,
    });
    assert.equal(result.status, 1, result.stderr);
    assert.ok(result.stderr.includes("OPENCODE_ROUTE_PROXY_TIMEOUT_MS"));
    assert.ok(result.stderr.includes(`"${raw}"`));
    assert.doesNotMatch(result.stdout, /listening/i);
  }
});

test("configured deadline aborts a slow leaf", async () => {
  const p = await startProxy("150");
  try {
    p.leafA.mode = "slow";
    const start = Date.now();
    const res = await p.post({ model: "opencode-route/t-variant", messages: MSGS });
    assert.ok(res.status >= 500, `unexpected status ${res.status}`);
    assert.ok(Date.now() - start < 3000, "deadline did not abort the slow leaf promptly");
    assert.equal(p.leafA.hits, 1);
    assert.equal(p.attempts()[0].outcome, "timeout");
  } finally {
    await p.cleanup();
  }
});

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

// Overnight windows (#3874): the fixture "overnight" window is
// Fri 22:00-02:00 UTC. 2026-11-06 is a Friday, 2026-11-07 a Saturday.
// The after-midnight leg belongs to the day the window opened.
const FRI_2300 = Date.UTC(2026, 10, 6, 23, 0);
const SAT_0100 = Date.UTC(2026, 10, 7, 1, 0);
const FRI_0100 = Date.UTC(2026, 10, 6, 1, 0);
const SAT_2300 = Date.UTC(2026, 10, 7, 23, 0);

test("overnight window is active on its opening evening", async () => {
  const p = await startProxy({ nowMs: FRI_2300 });
  try {
    const res = await p.post({ model: "opencode-route/t-window-overnight", messages: MSGS });
    assert.equal(res.status, 200);
    assert.equal((await res.json()).choices[0].message.content, "LEAF-A");
  } finally {
    await p.cleanup();
  }
});

test("overnight window stays active on the after-midnight leg", async () => {
  const p = await startProxy({ nowMs: SAT_0100 });
  try {
    const res = await p.post({ model: "opencode-route/t-window-overnight", messages: MSGS });
    assert.equal(res.status, 200);
    assert.equal((await res.json()).choices[0].message.content, "LEAF-A");
  } finally {
    await p.cleanup();
  }
});

test("overnight window is inactive on an unlisted day's after-midnight leg", async () => {
  const p = await startProxy({ nowMs: FRI_0100 });
  try {
    const res = await p.post({ model: "opencode-route/t-window-overnight", messages: MSGS });
    assert.equal(res.status, 200);
    assert.equal((await res.json()).choices[0].message.content, "LEAF-B");
  } finally {
    await p.cleanup();
  }
});

test("overnight window is inactive on an unlisted evening", async () => {
  const p = await startProxy({ nowMs: SAT_2300 });
  try {
    const res = await p.post({ model: "opencode-route/t-window-overnight", messages: MSGS });
    assert.equal(res.status, 200);
    assert.equal((await res.json()).choices[0].message.content, "LEAF-B");
  } finally {
    await p.cleanup();
  }
});

test("provider route (ordered) expands to member providers in group order", async () => {
  const p = await startProxy();
  try {
    const res = await p.post({ model: "opencode-route/t-group", messages: MSGS });
    assert.equal(res.status, 200);
    const json = await res.json();
    // grp-ordered lists leafB first; the attempt must be recorded
    // under the concrete member, never under the group id (#3256).
    assert.equal(json.choices[0].message.content, "LEAF-B");
    const rows = p.attempts();
    assert.equal(rows.length, 1);
    assert.equal(rows[0].provider_id, "leafB");
    assert.equal(rows[0].model_id, "m-g");
    assert.equal(rows[0].outcome, "success");
    assert.equal(rows[0].session_id, null);
  } finally {
    await p.cleanup();
  }
});

test("provider route (round-robin) rotates the starting member", async () => {
  const p = await startProxy();
  try {
    const first = await (await p.post({ model: "opencode-route/t-group-rr", messages: MSGS })).json();
    const second = await (await p.post({ model: "opencode-route/t-group-rr", messages: MSGS })).json();
    assert.equal(first.choices[0].message.content, "LEAF-A");
    assert.equal(second.choices[0].message.content, "LEAF-B");
    assert.deepEqual(
      p.attempts().map((r) => r.provider_id),
      ["leafA", "leafB"],
    );
  } finally {
    await p.cleanup();
  }
});

test("provider route (weighted) honors weights keyed by member provider id", async () => {
  const p = await startProxy();
  try {
    const res = await p.post({ model: "opencode-route/t-group-weighted", messages: MSGS });
    assert.equal(res.status, 200);
    const json = await res.json();
    // leafA carries weight 0 in grp-weighted, leafB weight 1.
    assert.equal(json.choices[0].message.content, "LEAF-B");
    assert.equal(p.attempts()[0].provider_id, "leafB");
  } finally {
    await p.cleanup();
  }
});

test("provider route skips a member whose catalog lacks the model", async () => {
  const p = await startProxy();
  try {
    const res = await p.post({ model: "opencode-route/t-group-skip", messages: MSGS });
    assert.equal(res.status, 200);
    const json = await res.json();
    // grp-ordered is [leafB, leafA], but only leafA offers m-a.
    assert.equal(json.choices[0].message.content, "LEAF-A");
    assert.deepEqual(
      p.attempts().map((r) => r.provider_id),
      ["leafA"],
    );
  } finally {
    await p.cleanup();
  }
});

test("empty-shell openai block resolves the built-in endpoint default", async () => {
  const p = await startProxy();
  try {
    // The fixture openai block is an empty shell (models only, no
    // npm/options/baseURL) and no credential is stored for it. Since
    // #3228, a credential-less leaf is skipped before any attempt, so
    // this route — whose only leaf is credential-less — fails with
    // the no-credentialed-members error and records NO attempt (the
    // pre-#3228 shape, one Authentication attempt, is superseded).
    const res = await p.post({ model: "opencode-route/t-openai-default", messages: MSGS });
    assert.equal(res.status, 502);
    const json = await res.json();
    assert.match(json.error.message, /no credentialed members/);
    assert.equal(p.attempts().length, 0);
  } finally {
    await p.cleanup();
  }
});

test("credential-less leaves are skipped in a walk (#3228)", async () => {
  const p = await startProxy();
  try {
    // leafNoCred's credential row is inactive; the walk must complete
    // via the healthy leafB and record no attempt against leafNoCred.
    const res = await p.post({ model: "opencode-route/t-nocred-first", messages: MSGS });
    assert.equal(res.status, 200);
    const json = await res.json();
    assert.equal(json.choices[0].message.content, "LEAF-B");
    assert.deepEqual(
      p.attempts().map((r) => r.provider_id),
      ["leafB"],
    );
  } finally {
    await p.cleanup();
  }
});

test("a walk of only credential-less leaves fails without an Authentication error (#3228)", async () => {
  const p = await startProxy();
  try {
    const res = await p.post({ model: "opencode-route/t-nocred-only", messages: MSGS });
    assert.equal(res.status, 502);
    const json = await res.json();
    assert.match(json.error.message, /no credentialed members/);
    assert.equal(p.attempts().length, 0);
  } finally {
    await p.cleanup();
  }
});

// Range checks (#4259): out-of-range window times fail closed (never
// active) as a window start and as a window end. Each case pins `now`
// where the old shape-only parseHHMM misread the window as active:
// 24:00-12:00 and 25:00-12:00 wrap into the after-midnight leg at
// 02:30, 09:00-24:60 / 09:00-25:00 cover 23:30 and 12:00, and
// 99:99-99:98 was always-on.
test("out-of-range window times fail closed as start and as end", async () => {
  const AT_0230 = Date.UTC(2026, 10, 4, 2, 30);
  const AT_1200 = Date.UTC(2026, 10, 4, 12, 0);
  const AT_2330 = Date.UTC(2026, 10, 4, 23, 30);
  const cases = [
    ["t-window-bad-start", AT_0230],
    ["t-window-bad-start2500", AT_0230],
    ["t-window-bad-wrap", AT_0230],
    ["t-window-bad-wrap", AT_1200],
    ["t-window-bad-wrap", AT_2330],
    ["t-window-bad-end", AT_2330],
    ["t-window-bad-end", AT_1200],
    ["t-window-bad-end2500", AT_1200],
    ["t-window-bad-end2500", AT_2330],
  ];
  for (const [route, nowMs] of cases) {
    const p = await startProxy({ nowMs });
    try {
      const res = await p.post({ model: `opencode-route/${route}`, messages: MSGS });
      assert.equal(res.status, 200, route);
      assert.equal((await res.json()).choices[0].message.content, "LEAF-B", `${route} at ${new Date(nowMs).toISOString()}`);
      assert.deepEqual(
        p.attempts().map((r) => r.provider_id),
        ["leafB"],
        route,
      );
    } finally {
      await p.cleanup();
    }
  }
});

test("boundary times 00:00 and 23:59 remain valid windows", async () => {
  const AT_0001 = Date.UTC(2026, 10, 4, 0, 1);
  const AT_2358 = Date.UTC(2026, 10, 4, 23, 58);
  const AT_1200 = Date.UTC(2026, 10, 4, 12, 0);
  const early = await startProxy({ nowMs: AT_0001 });
  try {
    const res = await early.post({ model: "opencode-route/t-window-edge-early", messages: MSGS });
    assert.equal((await res.json()).choices[0].message.content, "LEAF-A");
  } finally {
    await early.cleanup();
  }
  const late = await startProxy({ nowMs: AT_2358 });
  try {
    const res = await late.post({ model: "opencode-route/t-window-edge-late", messages: MSGS });
    assert.equal((await res.json()).choices[0].message.content, "LEAF-A");
  } finally {
    await late.cleanup();
  }
  const mid = await startProxy({ nowMs: AT_1200 });
  try {
    for (const route of ["t-window-edge-early", "t-window-edge-late"]) {
      const res = await mid.post({ model: `opencode-route/${route}`, messages: MSGS });
      assert.equal((await res.json()).choices[0].message.content, "LEAF-B", route);
    }
  } finally {
    await mid.cleanup();
  }
});

test("config load names invalid windows instead of silently evaluating them", async () => {
  const p = await startProxy({ nowMs: Date.UTC(2026, 10, 4, 2, 30) });
  try {
    await p.post({ model: "opencode-route/t-window-bad-wrap", messages: MSGS });
    let log = "";
    for (let i = 0; i < 20; i++) {
      log = p.log();
      if (log.includes("bad-wrap-99") && log.includes("99:99")) break;
      await new Promise((r) => setTimeout(r, 100));
    }
    assert.ok(log.includes("bad-wrap-99"), `log names the window:\n${log}`);
    assert.ok(log.includes("99:99"), `log names the bad value:\n${log}`);
    assert.ok(log.includes("bad-start-2400"), `log names each bad window:\n${log}`);
  } finally {
    await p.cleanup();
  }
});

// ------------------------------------------------------------ #5295 gate

// Seed one quota-bearing failure for a (provider, model) pair, as if
// the leaf had answered 429 with a Retry-After `retryAfterMs` long at
// `atMs`. Written straight into the fixture ledger the proxy reads.
const seedQuotaFailure = (dbPath, { provider, model, atMs, retryAfterMs, endedMs }) => {
  const db = new DatabaseSync(dbPath);
  db.prepare(
    `INSERT INTO route_attempt (
       time_started, time_ended, session_id, assistant_message_id,
       route_id, provider_id, model_id, variant, outcome,
       error_tag, error_code, error_status, error_message, retryable,
       output_started, failed_over_to, quota
     ) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)`,
  ).run(
    atMs,
    endedMs ?? atMs + 50,
    null,
    null,
    "seed",
    provider,
    model,
    null,
    "failure",
    "RateLimit",
    null,
    429,
    "leaf HTTP 429: quota",
    1,
    0,
    null,
    JSON.stringify({ retryAfterMs }),
  );
  db.close();
};

// A Monday noon UTC, pinned via OPENCODE_ROUTE_PROXY_NOW_MS so the
// gate's clock is deterministic.
const GATE_NOW = Date.UTC(2026, 0, 5, 12, 0, 0);

test("known-exhausted pair is excluded while a live pair exists (#5295)", async () => {
  const p = await startProxy({ nowMs: GATE_NOW });
  try {
    // One minute ago the leaf said: retry after an hour. The pair is
    // known exhausted for another ~59 minutes — one failure is
    // enough; demotion's >=3 must not be required.
    seedQuotaFailure(p.dbPath, {
      provider: "leafA",
      model: "m-a",
      atMs: GATE_NOW - 60_000,
      retryAfterMs: 3_600_000,
    });
    const res = await p.post({ model: "opencode-route/t-ordered", messages: MSGS });
    assert.equal(res.status, 200);
    const body = await res.json();
    assert.equal(body.choices[0].message.content, "LEAF-B");
    assert.equal(p.leafA.hits, 0, "the exhausted leaf must not be attempted");
    const rows = p.attempts().filter((r) => r.route_id === "t-ordered");
    assert.equal(rows.length, 1);
    assert.equal(rows[0].provider_id, "leafB");
    assert.ok(
      p.log().includes("known-exhausted gate excluded leafA/m-a"),
      `gate decision should be in the proxy diagnostics: ${p.log()}`,
    );
  } finally {
    await p.cleanup();
  }
});

test("known-exhausted pair returns after its reset passes (#5295)", async () => {
  const p = await startProxy({ nowMs: GATE_NOW });
  try {
    // Two hours ago the leaf said retry after an hour: the reset is
    // an hour in the past at the pinned now, so the gate has lifted.
    seedQuotaFailure(p.dbPath, {
      provider: "leafA",
      model: "m-a",
      atMs: GATE_NOW - 7_200_000,
      retryAfterMs: 3_600_000,
    });
    const res = await p.post({ model: "opencode-route/t-ordered", messages: MSGS });
    assert.equal(res.status, 200);
    const body = await res.json();
    assert.equal(body.choices[0].message.content, "LEAF-A");
    assert.equal(p.leafB.hits, 0);
  } finally {
    await p.cleanup();
  }
});

test("all candidates known-exhausted: route still walks in order (#5295)", async () => {
  const p = await startProxy({ nowMs: GATE_NOW });
  try {
    seedQuotaFailure(p.dbPath, {
      provider: "leafA",
      model: "m-a",
      atMs: GATE_NOW - 60_000,
      retryAfterMs: 3_600_000,
    });
    seedQuotaFailure(p.dbPath, {
      provider: "leafB",
      model: "m-b",
      atMs: GATE_NOW - 60_000,
      retryAfterMs: 3_600_000,
    });
    // Fail-safe: a known-exhausted leg as last resort beats no route,
    // so the walk keeps the (demoted) order and leaf A still serves.
    const res = await p.post({ model: "opencode-route/t-ordered", messages: MSGS });
    assert.equal(res.status, 200);
    const body = await res.json();
    assert.equal(body.choices[0].message.content, "LEAF-A");
    assert.ok(
      p.log().includes("known-exhausted gate bypassed"),
      `bypass should be in the proxy diagnostics: ${p.log()}`,
    );
  } finally {
    await p.cleanup();
  }
});

test("a success after the quota failure clears the gate (#5295)", async () => {
  const p = await startProxy({ nowMs: GATE_NOW });
  try {
    seedQuotaFailure(p.dbPath, {
      provider: "leafA",
      model: "m-a",
      atMs: GATE_NOW - 120_000,
      retryAfterMs: 3_600_000,
    });
    // The pair served successfully a minute after that failure
    // (another door sharing the ledger): the exhaustion is stale.
    const db = new DatabaseSync(p.dbPath);
    db.prepare(
      `INSERT INTO route_attempt (
         time_started, time_ended, session_id, assistant_message_id,
         route_id, provider_id, model_id, variant, outcome,
         error_tag, error_code, error_status, error_message, retryable,
         output_started, failed_over_to, quota
       ) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)`,
    ).run(
      GATE_NOW - 60_000,
      GATE_NOW - 59_000,
      null,
      null,
      "seed",
      "leafA",
      "m-a",
      null,
      "success",
      null,
      null,
      null,
      null,
      1,
      0,
      null,
      null,
    );
    db.close();
    const res = await p.post({ model: "opencode-route/t-ordered", messages: MSGS });
    assert.equal(res.status, 200);
    const body = await res.json();
    assert.equal(body.choices[0].message.content, "LEAF-A");
  } finally {
    await p.cleanup();
  }
});

test("gate bypassed when the only survivor has no credential (#5295)", async () => {
  const p = await startProxy({ nowMs: GATE_NOW });
  try {
    seedQuotaFailure(p.dbPath, {
      provider: "leafA",
      model: "m-a",
      atMs: GATE_NOW - 60_000,
      retryAfterMs: 3_600_000,
    });
    // leafA/m-a is known-exhausted and leafNoCred/m-b is not — but
    // leafNoCred has no credential, so excluding leafA would still
    // empty the route at the #3228 filter. The gate must bypass and
    // keep leafA as the last resort instead of failing the request.
    const res = await p.post({ model: "opencode-route/t-gate-nocred", messages: MSGS });
    assert.equal(res.status, 200);
    const body = await res.json();
    assert.equal(body.choices[0].message.content, "LEAF-A");
    assert.ok(
      p.log().includes("known-exhausted gate bypassed"),
      `bypass should be in the proxy diagnostics: ${p.log()}`,
    );
  } finally {
    await p.cleanup();
  }
});

test("retry delay is anchored to the quota response receipt (#5295)", async () => {
  const p = await startProxy({ nowMs: GATE_NOW });
  try {
    // The 429 took 90s to arrive (started 2 minutes ago, received 30s
    // ago) and asked for 60 more seconds of backoff. Anchored to the
    // request start the pair would already be eligible again
    // (start + 60s < now); anchored to the receipt it is exhausted
    // for another ~30s.
    seedQuotaFailure(p.dbPath, {
      provider: "leafA",
      model: "m-a",
      atMs: GATE_NOW - 120_000,
      endedMs: GATE_NOW - 30_000,
      retryAfterMs: 60_000,
    });
    const res = await p.post({ model: "opencode-route/t-ordered", messages: MSGS });
    assert.equal(res.status, 200);
    const body = await res.json();
    assert.equal(body.choices[0].message.content, "LEAF-B");
    assert.equal(p.leafA.hits, 0);
  } finally {
    await p.cleanup();
  }
});

test("an overlapping success completed before the quota response does not clear the gate (#5295)", async () => {
  const p = await startProxy({ nowMs: GATE_NOW });
  try {
    // Quota failure: started 2 minutes ago, the 429 arrived 10s ago.
    seedQuotaFailure(p.dbPath, {
      provider: "leafA",
      model: "m-a",
      atMs: GATE_NOW - 120_000,
      endedMs: GATE_NOW - 10_000,
      retryAfterMs: 3_600_000,
    });
    // Overlapping success: STARTED after the failing request (so
    // start-time ordering would call it the latest word) but
    // COMPLETED a minute ago — before the 429 was received. It is
    // not a recovery; the gate must hold.
    const db = new DatabaseSync(p.dbPath);
    db.prepare(
      `INSERT INTO route_attempt (
         time_started, time_ended, session_id, assistant_message_id,
         route_id, provider_id, model_id, variant, outcome,
         error_tag, error_code, error_status, error_message, retryable,
         output_started, failed_over_to, quota
       ) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)`,
    ).run(
      GATE_NOW - 110_000,
      GATE_NOW - 60_000,
      null,
      null,
      "seed",
      "leafA",
      "m-a",
      null,
      "success",
      null,
      null,
      null,
      null,
      1,
      0,
      null,
      null,
    );
    db.close();
    const res = await p.post({ model: "opencode-route/t-ordered", messages: MSGS });
    assert.equal(res.status, 200);
    const body = await res.json();
    assert.equal(body.choices[0].message.content, "LEAF-B");
    assert.equal(p.leafA.hits, 0);
  } finally {
    await p.cleanup();
  }
});
