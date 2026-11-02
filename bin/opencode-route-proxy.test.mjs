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
    },
    "model_routes": {
      "t-ordered": { "selection": "ordered", "targets": ["leafA/m-a", "leafB/m-b"] },
      "t-variant": { "selection": "ordered", "targets": [ { "model": "leafA/m-a", "variants": { "medium": "high" } } ] },
      "t-nested": { "selection": "ordered", "targets": ["opencode-route/t-ordered"] },
      "t-disabled": { "selection": "ordered", "targets": ["leafOff/m-b", "leafB/m-b"] },
      "t-window": { "selection": "ordered", "targets": [ { "model": "leafA/m-a", "window": "peak" }, "leafB/m-b" ] },
      "t-window-inv": { "selection": "ordered", "targets": [ { "model": "leafA/m-a", "window": { "name": "peak", "invert": true } }, "leafB/m-b" ] },
      "t-window-unknown": { "selection": "ordered", "targets": [ { "model": "leafA/m-a", "window": "no-such-window" }, "leafB/m-b" ] },
      "t-window-overnight": { "selection": "ordered", "targets": [ { "model": "leafA/m-a", "window": "overnight" }, "leafB/m-b" ] },
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
  return { base, post, attempts, leafA, leafB, cleanup };
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
    // npm/options/baseURL) and no credential is stored for it, so a
    // resolved endpoint surfaces as a credential failure — while an
    // unresolved one would surface as UnknownProvider (#3256). The
    // check order in callLeaf (endpoint first, credential second)
    // makes this a network-free proof the default resolved.
    const res = await p.post({ model: "opencode-route/t-openai-default", messages: MSGS });
    assert.equal(res.status, 502);
    const rows = p.attempts();
    assert.equal(rows.length, 1);
    assert.equal(rows[0].provider_id, "openai");
    assert.equal(rows[0].error_tag, "Authentication");
    assert.notEqual(rows[0].error_tag, "UnknownProvider");
  } finally {
    await p.cleanup();
  }
});
