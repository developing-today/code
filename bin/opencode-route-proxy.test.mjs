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
import { once } from "node:events";
import fs from "node:fs";
import { createServer } from "node:http";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";
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
    mode: "ok", // ok | rate | rate-body-reset | auth | sse-rate | delayed-sse-rate | stream-break | late-sse-rate | delayed-stream | big-first-event | preface-flood | huge-body | huge-body-chunked
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
      const bearerOk = req.headers.authorization === `Bearer ${expectedKey}`;
      const googOk = req.headers["x-goog-api-key"] === expectedKey;
      if (!bearerOk && !googOk) {
        res.writeHead(401, { "content-type": "application/json" });
        return res.end(JSON.stringify({ error: { message: "bad key" } }));
      }
      if (leaf.mode === "auth") {
        res.writeHead(401, { "content-type": "application/json" });
        return res.end(
          JSON.stringify({ error: { message: "expired credential" } }),
        );
      }
      if (leaf.mode === "rate") {
        res.writeHead(429, {
          "content-type": "application/json",
          "retry-after": "30",
        });
        return res.end(JSON.stringify({ error: { message: "quota" } }));
      }
      if (leaf.mode === "rate-body-reset") {
        // A quota wall that names its reset in the BODY with no
        // Retry-After header (the _muse shape in code#9859): the
        // reset instant must still wall the leaf (#12228).
        res.writeHead(429, { "content-type": "application/json" });
        const resetAt = new Date(Date.now() + 3600 * 1000).toISOString();
        return res.end(
          JSON.stringify({
            error: {
              message: `Subscription quota exhausted. Your usage window resets at ${resetAt}.`,
            },
          }),
        );
      }
      if (leaf.mode === "rate-http-date") {
        // Retry-After as an HTTP-date, ~1h out (#13693 RC-1).
        res.writeHead(429, {
          "content-type": "application/json",
          "retry-after": new Date(Date.now() + 3600 * 1000).toUTCString(),
        });
        return res.end(JSON.stringify({ error: { message: "quota" } }));
      }
      if (leaf.mode === "rate-prose") {
        // No header; the body names only a duration in prose (#13693).
        res.writeHead(429, { "content-type": "application/json" });
        return res.end(
          JSON.stringify({
            error: {
              message: "Quota exceeded. Please try again in 20 minutes.",
            },
          }),
        );
      }
      if (leaf.mode === "rate-json-retry-after") {
        // No header; the body carries {"retry_after": 30} (#13693).
        res.writeHead(429, { "content-type": "application/json" });
        return res.end(
          JSON.stringify({ error: { message: "quota", retry_after: 30 } }),
        );
      }
      if (leaf.mode === "rate-json-retry-delay") {
        // No header; the body carries {"retryDelay": "30s"}, the
        // Google-style pure-duration string (#13693).
        res.writeHead(429, { "content-type": "application/json" });
        return res.end(
          JSON.stringify({ error: { message: "quota", retryDelay: "30s" } }),
        );
      }
      if (leaf.mode === "rate-x-reset") {
        // No Retry-After; an x-ratelimit-reset epoch-seconds header
        // ~1h out carries the reset (#13693 RC-1).
        res.writeHead(429, {
          "content-type": "application/json",
          "x-ratelimit-reset": String(
            Math.floor((Date.now() + 3600 * 1000) / 1000),
          ),
        });
        return res.end(JSON.stringify({ error: { message: "quota" } }));
      }
      if (leaf.mode === "sse-rate") {
        res.writeHead(200, {
          "content-type": "text/event-stream",
          "retry-after": "30",
        });
        const error = JSON.stringify({
          error: {
            message: "quota exhausted",
            type: "rate_limit_error",
            code: "insufficient_quota",
            status: 429,
          },
        });
        res.write(`data: ${error.slice(0, 14)}`);
        return setTimeout(() => res.end(`${error.slice(14)}\n\n`), 10);
      }
      if (leaf.mode === "delayed-sse-rate") {
        res.writeHead(200, {
          "content-type": "text/event-stream",
          "retry-after": "30",
        });
        res.write(": heartbeat\n\n");
        const error = JSON.stringify({
          error: {
            message: "quota exhausted",
            type: "rate_limit_error",
            code: "insufficient_quota",
            status: 429,
          },
        });
        return setTimeout(() => {
          res.write(`data: ${error.slice(0, 14)}`);
          setTimeout(() => res.end(`${error.slice(14)}\n\n`), 10);
        }, 300);
      }
      if (leaf.mode === "stream-break") {
        res.writeHead(200, { "content-type": "text/event-stream" });
        res.write(": heartbeat\n\n");
        return setTimeout(() => res.destroy(), 10);
      }
      if (leaf.mode === "embed-400") {
        res.writeHead(400, { "content-type": "application/json" });
        return res.end(JSON.stringify({ error: { message: "bad embedding request" } }));
      }
      if (req.url.includes(":batchEmbedContent")) {
        // Google native batch shape.
        const reqs = Array.isArray(body.requests) ? body.requests : [];
        if (reqs.some((r) => typeof r?.content?.parts?.[0]?.text !== "string")) {
          res.writeHead(400, { "content-type": "application/json" });
          return res.end(JSON.stringify({ error: { message: "bad batch request" } }));
        }
        if (
          leaf.mode === "embed-bad-item" &&
          reqs.some((r) => String(r.content.parts[0].text).includes("bad"))
        ) {
          res.writeHead(400, { "content-type": "application/json" });
          return res.end(JSON.stringify({ error: { message: "bad item" } }));
        }
        res.writeHead(200, { "content-type": "application/json" });
        return res.end(
          JSON.stringify({
            embeddings: reqs.map((r) => ({
              values: [r.content.parts[0].text.length, 0.5],
            })),
          }),
        );
      }
      if (req.url.includes(":embedContent")) {
        // Google native single shape.
        const text = body?.content?.parts?.[0]?.text;
        if (typeof text !== "string") {
          res.writeHead(400, { "content-type": "application/json" });
          return res.end(JSON.stringify({ error: { message: "bad embed request" } }));
        }
        res.writeHead(200, { "content-type": "application/json" });
        return res.end(
          JSON.stringify({ embedding: { values: [text.length, 0.5] } }),
        );
      }
      if (req.url.includes("/embeddings")) {
        const input = body.input;
        const texts = typeof input === "string" ? [input] : input;
        if (
          leaf.mode === "embed-bad-item" &&
          Array.isArray(texts) &&
          texts.some((t) => typeof t === "string" && t.includes("bad"))
        ) {
          res.writeHead(400, { "content-type": "application/json" });
          return res.end(JSON.stringify({ error: { message: "bad item" } }));
        }
        if (typeof input === "string") {
          res.writeHead(200, { "content-type": "application/json" });
          return res.end(
            JSON.stringify({
              object: "list",
              data: [{ object: "embedding", index: 0, embedding: [input.length, 0.5] }],
              model: body.model,
            }),
          );
        }
        if (Array.isArray(input) && input.every((t) => typeof t === "string")) {
          res.writeHead(200, { "content-type": "application/json" });
          return res.end(
            JSON.stringify({
              object: "list",
              data: input.map((t, i) => ({
                object: "embedding",
                index: i,
                embedding: [t.length, 0.5],
              })),
              model: body.model,
            }),
          );
        }
        res.writeHead(400, { "content-type": "application/json" });
        return res.end(JSON.stringify({ error: { message: "bad embedding input" } }));
      }
      if (req.url.includes("/decisions")) {
        if (leaf.mode === "dec-400") {
          res.writeHead(400, { "content-type": "application/json" });
          return res.end(JSON.stringify({ error: { message: "bad decision" } }));
        }
        res.writeHead(200, { "content-type": "application/json" });
        return res.end(
          JSON.stringify({ decision: "yes", model: body.model, echo: body.input ?? null }),
        );
      }
      if (req.url.includes("/rerank")) {
        res.writeHead(200, { "content-type": "application/json" });
        const docs = Array.isArray(body.documents) ? body.documents : [];
        return res.end(
          JSON.stringify({
            results: docs.map((d, i) => ({ index: i, score: 1 - i * 0.1, document: d })),
            model: body.model,
          }),
        );
      }
      if (req.url.includes("/audio/transcriptions")) {
        res.writeHead(200, { "content-type": "application/json" });
        return res.end(JSON.stringify({ text: `transcribed-by-${name}` }));
      }
      if (req.url.includes("/audio/speech")) {
        const audio = Buffer.from(`audio-by-${name}`, "utf8");
        res.writeHead(200, { "content-type": "audio/mpeg", "content-length": audio.length });
        return res.end(audio);
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
        if (leaf.mode === "big-first-event") {
          // One valid first content event larger than the relay's
          // 64 KiB preface window, delivered in slices so the parser
          // buffer crosses the window mid-event (#5558 Codex P2).
          const big = "A".repeat(96 * 1024);
          const event = `data: ${JSON.stringify({
            id: "chatcmpl-x",
            object: "chat.completion.chunk",
            created: 1,
            model: body.model,
            choices: [
              {
                index: 0,
                delta: {
                  tool_calls: [
                    { index: 0, function: { name: "f", arguments: big } },
                  ],
                },
                finish_reason: null,
              },
            ],
          })}\n\n`;
          let off = 0;
          const step = () => {
            if (off < event.length) {
              res.write(event.slice(off, off + 8192));
              off += 8192;
              return setTimeout(step, 1);
            }
            res.write(
              chunk({}, { prompt_tokens: 5, completion_tokens: 2, total_tokens: 7 }),
            );
            return res.end("data: [DONE]\n\n");
          };
          return step();
        }
        if (leaf.mode === "preface-flood") {
          // A non-content preface past the relay's pre-content
          // limit, then the response stays open: the relay must
          // fail over AND cancel this stream (#5558 Codex P2).
          // Cancellation is observable here as the socket closing
          // before the response ever ended.
          leaf.prefaceAborted = false;
          res.on("close", () => {
            if (!res.writableEnded) leaf.prefaceAborted = true;
          });
          res.write(": preface\n\n");
          let sent = 0;
          const step = () => {
            if (res.destroyed || res.writableEnded) return;
            res.write(`: ${"p".repeat(4096)}\n\n`);
            sent += 4096;
            if (sent < 192 * 1024) return setTimeout(step, 1);
            // Hold the response open; only the relay's cancel ends it.
          };
          return step();
        }
        if (leaf.mode === "delayed-stream") {
          res.write(chunk({ role: "assistant" }));
          return setTimeout(() => {
            res.write(chunk({ content: name }));
            res.write(
              chunk({}, { prompt_tokens: 5, completion_tokens: 2, total_tokens: 7 }),
            );
            res.end("data: [DONE]\n\n");
          }, 350);
        }
        res.write(chunk({ role: "assistant" }));
        res.write(chunk({ content: name }));
        if (leaf.mode === "late-sse-rate") {
          return res.end(
            `data: ${JSON.stringify({
              error: {
                message: "late quota exhausted",
                type: "rate_limit_error",
                code: "insufficient_quota",
                status: 429,
              },
            })}\n\n`,
          );
        }
        res.write(
          chunk({}, { prompt_tokens: 5, completion_tokens: 2, total_tokens: 7 }),
        );
        return res.end("data: [DONE]\n\n");
      }
      if (leaf.mode === "huge-body") {
        // A valid completion whose body exceeds the leaf-body cap,
        // with an honest content-length (#16398).
        const big = JSON.stringify({
          id: "chatcmpl-x",
          object: "chat.completion",
          created: 1,
          model: body.model,
          choices: [
            {
              index: 0,
              finish_reason: "stop",
              message: { role: "assistant", content: "x".repeat(8192) },
            },
          ],
        });
        res.writeHead(200, {
          "content-type": "application/json",
          "content-length": Buffer.byteLength(big),
        });
        return res.end(big);
      }
      if (leaf.mode === "huge-body-chunked") {
        // An over-cap body with NO content-length, dribbled in
        // chunks: only a streaming byte cap can stop it (#16398).
        res.writeHead(200, { "content-type": "application/json" });
        const piece = "x".repeat(1024);
        for (let i = 0; i < 8; i++) res.write(piece);
        return res.end();
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
  const timeout =
    typeof arg === "string" ? arg : String(arg.timeoutMs ?? "15000");
  const leafTimeout = typeof arg === "object" ? arg.leafTimeoutMs : undefined;
  const leafBodyMax =
    typeof arg === "object" ? arg.leafBodyMaxBytes : undefined;
  const pinnedNow = arg && typeof arg === "object" ? arg.nowMs : undefined;
  const systemdActivation =
    arg && typeof arg === "object" && arg.systemdActivation === true;
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
    "leafG": {
      "npm": "@ai-sdk/openai-compatible",
      "options": {
        "baseURL": "http://127.0.0.1:${leafA.port}/v1beta",
        "apiKey": "key-a",
        "endpoints": {
          "embed": { "kind": "google-embed", "path": "/models/{model}:embedContent" },
          "decision": { "kind": "google-decisions", "path": "/decisions" }
        }
      },
      "models": { "m-ge": {}, "m-gd": {} },
    },
    "leafGsplit": {
      "npm": "@ai-sdk/openai-compatible",
      "options": {
        "baseURL": "http://127.0.0.1:${leafA.port}/v1beta",
        "apiKey": "key-a",
        "endpoints": {
          "embed": { "kind": "google-embed", "path": "/models/{model}:embedContent", "maxBatchItems": 2 }
        }
      },
      "models": { "m-ge": {} },
    },
    "leafZ": {
      "npm": "@ai-sdk/openai-compatible",
      "options": {
        "baseURL": "http://127.0.0.1:${leafA.port}/v1",
        "apiKey": "key-a",
        "endpoints": {
          "decision": { "kind": "jev-systemone", "path": "/decisions" }
        }
      },
      "models": { "m-zd": {} },
    },
    "leafF": {
      "npm": "@ai-sdk/openai-compatible",
      "options": {
        "baseURL": "http://127.0.0.1:${leafA.port}/v1",
        "apiKey": "key-a",
        "endpoints": {
          "decision": { "kind": "fireworks", "path": "/decisions" }
        }
      },
      "models": { "m-fd": {} },
    },
    "leafWeird": {
      "npm": "@ai-sdk/openai-compatible",
      "options": {
        "baseURL": "http://127.0.0.1:${leafA.port}/v1",
        "apiKey": "key-a",
        "endpoints": {
          "decision": { "kind": "not-a-real-kind", "path": "/decisions" }
        }
      },
      "models": { "m-wd": {} },
    },
    "leafFree": {
      "npm": "@ai-sdk/openai-compatible",
      "options": { "baseURL": "http://127.0.0.1:${leafA.port}/v1", "apiKey": "key-a" },
      "tokenMetering": {
        "groups": {
          "big": { "dailyTokens": 28, "marginTokens": 7, "models": ["m-free"] },
          "small": { "dailyTokens": 700, "marginTokens": 0, "models": ["m-free2"] }
        }
      },
      "models": { "m-free": {}, "m-free2": {} },
    },
    "leafPaid": {
      "npm": "@ai-sdk/openai-compatible",
      "options": { "baseURL": "http://127.0.0.1:${leafB.port}/v1", "apiKey": "key-b" },
      "models": { "m-free": {} },
    },
  },
  "model_designations": {
    "m-desig-dec": { "purposes": ["decision"] },
    "m-desig-chat": { "purposes": ["chat"] }
  },
  "experimental": {
    "provider_routes": {
      "grp-ordered": { "providers": ["leafB", "leafA"], "selection": "ordered" },
      "grp-rr": { "providers": ["leafA", "leafB"], "selection": "round-robin" },
      "grp-weighted": { "providers": ["leafA", "leafB"], "selection": "weighted", "weights": { "leafA": 0, "leafB": 1 } },
      "grp-prototype-spellings": { "providers": ["constructor", "__proto__", "toString", "hasOwnProperty", "valueOf", "leafA"], "selection": "ordered" },
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
      "bad-tz": { "start": "00:00", "end": "23:59", "timezone": "America/Chicagoo" },
      "bad-days-string": { "daysOfWeek": "fri", "start": "00:00", "end": "23:59", "timezone": "UTC" },
      "bad-days-name": { "daysOfWeek": ["wed", "funday"], "start": "00:00", "end": "23:59", "timezone": "UTC" },
      "full-day-names": { "daysOfWeek": ["Wednesday"], "start": "00:00", "end": "23:59", "timezone": "UTC" },
      "offpeak-x": { "daysOfWeek": ["mon", "tue", "wed", "thu", "fri"], "start": "04:00", "end": "06:00", "timezone": "UTC", "class": "off-peak" },
      "preburn-overlap": { "daysOfWeek": ["wed"], "start": "02:00", "end": "03:00", "timezone": "UTC", "class": "preburn" },
      "badclass-x": { "daysOfWeek": ["mon"], "start": "00:00", "end": "23:59", "timezone": "UTC", "class": "spicy" },
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
      "t-window-bad-tz": { "selection": "ordered", "targets": [ { "model": "leafA/m-a", "window": "bad-tz" }, "leafB/m-b" ] },
      "t-window-bad-days-string": { "selection": "ordered", "targets": [ { "model": "leafA/m-a", "window": "bad-days-string" }, "leafB/m-b" ] },
      "t-window-bad-days-name": { "selection": "ordered", "targets": [ { "model": "leafA/m-a", "window": "bad-days-name" }, "leafB/m-b" ] },
      "t-window-full-day-names": { "selection": "ordered", "targets": [ { "model": "leafA/m-a", "window": "full-day-names" }, "leafB/m-b" ] },
      "t-window-bound": { "selection": "ordered", "windows": ["peak"], "targets": [ { "model": "leafA/m-a", "window": "peak" }, "leafB/m-b" ] },
      "t-window-bound-multi": { "selection": "ordered", "windows": ["peak", "offpeak-x"], "targets": [ { "model": "leafA/m-a", "window": "offpeak-x" }, "leafB/m-b" ] },
      "t-window-badclass": { "selection": "ordered", "targets": [ { "model": "leafA/m-a", "window": "badclass-x" }, "leafB/m-b" ] },
      "t-group": { "selection": "ordered", "targets": ["grp-ordered/m-g"] },
      "t-group-rr": { "selection": "ordered", "targets": ["grp-rr/m-g"] },
      "t-group-weighted": { "selection": "ordered", "targets": ["grp-weighted/m-g"] },
      "t-group-prototype-spellings": { "selection": "ordered", "targets": ["grp-prototype-spellings/m-a"] },
      "t-group-skip": { "selection": "ordered", "targets": ["grp-ordered/m-a"] },
      "t-openai-default": { "selection": "ordered", "targets": ["openai/m-gpt"] },
      "t-a-only": { "selection": "ordered", "targets": ["leafA/m-a"] },
      "t-decide": { "purpose": "decision", "selection": "ordered", "targets": ["leafA/m-a", "leafB/m-b"] },
      "t-decide-goog": { "purpose": "decision", "selection": "ordered", "targets": ["leafG/m-gd"] },
      "t-decide-zen": { "purpose": "decision", "selection": "ordered", "targets": ["leafZ/m-zd"] },
      "t-decide-fw": { "purpose": "decision", "selection": "ordered", "targets": ["leafF/m-fd"] },
      "t-decide-weird": { "purpose": "decision", "selection": "ordered", "targets": ["leafWeird/m-wd"] },
      "t-decide-desig": { "purpose": "decision", "selection": "ordered", "targets": ["leafA/m-desig-dec", "leafB/m-desig-chat"] },
      "t-embed": { "purpose": "embed", "selection": "ordered", "targets": ["leafA/m-a", "leafB/m-b"] },
      "t-embed-goog": { "purpose": "embed", "selection": "ordered", "targets": ["leafG/m-ge"] },
      "t-embed-split": { "purpose": "embed", "selection": "ordered", "targets": ["leafGsplit/m-ge"] },
      "t-rerank": { "purpose": "rerank", "selection": "ordered", "targets": ["leafA/m-a", "leafB/m-b"] },
      "t-transcribe": { "purpose": "transcribe", "selection": "ordered", "targets": ["leafA/m-a", "leafB/m-b"] },
      "t-speak": { "purpose": "speak", "selection": "ordered", "targets": ["leafA/m-a", "leafB/m-b"] },
      "t-guard": { "purpose": "guard", "selection": "ordered", "targets": ["leafA/m-a", "leafB/m-b"] },
      "t-chat-desig": { "selection": "ordered", "targets": ["leafA/m-desig-dec", "leafB/m-b"] },
      "t-metered": { "selection": "ordered", "targets": ["leafFree/m-free", "leafPaid/m-free"] },
      "t-metered-only": { "selection": "ordered", "targets": ["leafFree/m-free"] },
      "t-metered-groups": { "selection": "ordered", "targets": ["leafFree/m-free", "leafFree/m-free2", "leafPaid/m-free"] },
    },
  },
}`,
  );

  let activationSocket;
  let activationFd;
  let port;
  if (systemdActivation) {
    activationSocket = net.createServer();
    await new Promise((resolve, reject) => {
      activationSocket.once("error", reject);
      activationSocket.listen(0, "127.0.0.1", resolve);
    });
    port = activationSocket.address().port;
    activationFd = activationSocket._handle.fd;
  } else {
    port = await freePort();
  }
  const env = {
    ...process.env,
    HOME: tmp,
    OPENCODE_ROUTE_PROXY_PORT: String(port),
    OPENCODE_ROUTE_PROXY_CONFIG: configPath,
    OPENCODE_ROUTE_PROXY_DB: dbPath,
    OPENCODE_ROUTE_PROXY_TIMEOUT_MS: timeout,
    ...(leafTimeout !== undefined
      ? { OPENCODE_ROUTE_PROXY_LEAF_TIMEOUT_MS: String(leafTimeout) }
      : {}),
    ...(leafBodyMax !== undefined
      ? { OPENCODE_ROUTE_PROXY_LEAF_BODY_MAX_BYTES: String(leafBodyMax) }
      : {}),
    ...(pinnedNow
      ? { OPENCODE_ROUTE_PROXY_NOW_MS: String(pinnedNow) }
      : {}),
  };
  delete env.LISTEN_FDS;
  delete env.LISTEN_PID;
  const proxyPath = path.join(binDir, "opencode-route-proxy.mjs");
  const childArgs = systemdActivation
    ? [
        "--input-type=module",
        "-e",
        `process.env.LISTEN_PID = String(process.pid); await import(${JSON.stringify(pathToFileURL(proxyPath).href)});`,
      ]
    : [proxyPath];
  if (systemdActivation) env.LISTEN_FDS = "1";
  const child = spawn(
    process.execPath,
    childArgs,
    {
      env,
      stdio: systemdActivation
        ? ["ignore", "pipe", "pipe", activationFd]
        : ["ignore", "pipe", "pipe"],
    },
  );
  if (systemdActivation) {
    await once(child, "spawn");
    await new Promise((resolve) => activationSocket.close(resolve));
  }
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
        "SELECT route_id, provider_id, model_id, variant, outcome, error_tag, error_status, error_message, quota, session_id, failed_over_to, output_started, first_token_ms FROM route_attempt ORDER BY id",
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
    if (activationSocket?.listening) {
      await new Promise((resolve) => activationSocket.close(resolve));
    }
    leafA.server.close();
    leafB.server.close();
  };
  return { base, post, attempts, leafA, leafB, cleanup, dbPath, log: () => log };
};

const MSGS = [
  { role: "system", content: "be terse" },
  { role: "user", content: "hello" },
];

const waitForAttempts = async (p, count) => {
  let rows = [];
  for (let i = 0; i < 20; i++) {
    try {
      rows = p.attempts();
    } catch {}
    if (rows.length >= count) break;
    await new Promise((r) => setTimeout(r, 100));
  }
  return rows;
};

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

test(
  "per-leaf first-response timeout fails over and records the attempt",
  async () => {
    const p = await startProxy({ timeoutMs: "500", leafTimeoutMs: "100" });
    try {
      p.leafA.mode = "slow";
      const start = Date.now();
      const res = await p.post({
        model: "opencode-route/t-ordered",
        messages: MSGS,
      });
      assert.equal(res.status, 200);
      const json = await res.json();
      assert.equal(json.choices[0].message.content, "LEAF-B");
      assert.ok(
        Date.now() - start < 2500,
        "per-leaf timeout did not shed promptly",
      );
      assert.equal(p.leafA.hits, 1);
      assert.equal(p.leafB.hits, 1);
      const rows = p.attempts();
      assert.equal(rows.length, 2);
      assert.equal(rows[0].outcome, "timeout");
      assert.equal(rows[0].error_tag, "Transport");
      assert.equal(rows[0].failed_over_to, "leafB/m-b");
      assert.equal(rows[1].outcome, "success");
    } finally {
      await p.cleanup();
    }
  },
);

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

test(
  "systemd LISTEN_FDS accepts on the inherited fd 3 listener (oracle: /health returns 200)",
  async () => {
    const p = await startProxy({ systemdActivation: true });
    try {
      const response = await fetch(`${p.base}/health`);
      assert.equal(response.status, 200);
      assert.deepEqual(await response.json(), {
        ok: true,
        service: "opencode-route-proxy",
      });
    } finally {
      await p.cleanup();
    }
  },
);

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
    const rows = await waitForAttempts(p, 2);
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

test("over-cap leaf body with a declared length fails over; both attempts recorded (#16398)", async () => {
  const p = await startProxy({ leafBodyMaxBytes: "1024" });
  try {
    p.leafA.mode = "huge-body";
    const res = await p.post({ model: "opencode-route/t-ordered", messages: MSGS });
    assert.equal(res.status, 200);
    const json = await res.json();
    assert.equal(json.choices[0].message.content, "LEAF-B");
    const rows = await waitForAttempts(p, 2);
    assert.equal(rows.length, 2);
    assert.equal(rows[0].outcome, "failure");
    assert.equal(rows[0].error_tag, "ProviderInternal");
    assert.match(rows[0].error_message, /exceeds the 1024-byte cap/);
    assert.equal(rows[0].failed_over_to, "leafB/m-b");
    assert.equal(rows[0].session_id, null);
    assert.equal(rows[1].outcome, "success");
  } finally {
    await p.cleanup();
  }
});

test("over-cap leaf body streamed without a length fails over (#16398)", async () => {
  const p = await startProxy({ leafBodyMaxBytes: "1024" });
  try {
    p.leafA.mode = "huge-body-chunked";
    const res = await p.post({ model: "opencode-route/t-ordered", messages: MSGS });
    assert.equal(res.status, 200);
    const json = await res.json();
    assert.equal(json.choices[0].message.content, "LEAF-B");
    const rows = await waitForAttempts(p, 2);
    assert.equal(rows.length, 2);
    assert.equal(rows[0].outcome, "failure");
    assert.equal(rows[0].error_tag, "ProviderInternal");
    assert.match(rows[0].error_message, /exceeds the 1024-byte cap/);
    assert.equal(rows[0].failed_over_to, "leafB/m-b");
    assert.equal(rows[1].outcome, "success");
  } finally {
    await p.cleanup();
  }
});

test("429 naming a body reset walls the leaf for later sessions (#12228)", async () => {
  const p = await startProxy();
  try {
    p.leafA.mode = "rate-body-reset";
    // The first session fails over to leafB ...
    const first = await p.post({ model: "opencode-route/t-ordered", messages: MSGS });
    assert.equal(first.status, 200);
    assert.equal((await first.json()).choices[0].message.content, "LEAF-B");
    // ... and the 429's body-carried reset is recorded as quota detail.
    const rows = await waitForAttempts(p, 2);
    assert.equal(rows.length, 2);
    assert.equal(rows[0].error_tag, "RateLimit");
    assert.notEqual(rows[0].quota, null);
    const quota = JSON.parse(rows[0].quota);
    const resetMs = Date.parse(quota.resetsAt ?? quota.resetAt ?? quota.retryAt ?? NaN);
    assert.ok(
      Number.isFinite(resetMs) && resetMs > Date.now(),
      `body reset not recorded as a future instant: ${rows[0].quota}`,
    );
    // A later session must not touch the walled leaf before its reset.
    const second = await p.post({ model: "opencode-route/t-ordered", messages: MSGS });
    assert.equal(second.status, 200);
    assert.equal((await second.json()).choices[0].message.content, "LEAF-B");
    assert.equal(p.leafA.hits, 1);
  } finally {
    await p.cleanup();
  }
});

test("an authentication failure benches a leaf until a probe succeeds (#12228)", async () => {
  const p = await startProxy();
  try {
    p.leafA.mode = "auth";
    // The first session fails over to leafB, and the 401 is recorded.
    const first = await p.post({ model: "opencode-route/t-ordered", messages: MSGS });
    assert.equal(first.status, 200);
    assert.equal((await first.json()).choices[0].message.content, "LEAF-B");
    const authFailures = p.attempts().filter((row) => row.provider_id === "leafA");
    assert.equal(authFailures.length, 1);
    assert.ok(authFailures.every((row) => row.outcome === "failure"));
    assert.ok(authFailures.every((row) => row.error_tag === "Authentication"));
    assert.ok(authFailures.every((row) => row.error_status === 401));
    assert.ok(authFailures.every((row) => row.quota === null));

    // Later sessions must not be routed to the dead door while its
    // bench holds: leafA sees no second attempt, and the bench says
    // so on the diagnostics stream.
    const second = await p.post({ model: "opencode-route/t-ordered", messages: MSGS });
    assert.equal(second.status, 200);
    assert.equal((await second.json()).choices[0].message.content, "LEAF-B");
    assert.equal(p.leafA.hits, 1);
    assert.ok(
      p.log().includes("auth bench excluded leafA/m-a"),
      `bench decision should be in the proxy diagnostics: ${p.log()}`,
    );

    // A successful probe re-admits the leaf. leafA has recovered; a
    // single-target request through it IS the probe (the bench's
    // fail-safe keeps a benched last resort as the only candidate).
    p.leafA.mode = "ok";
    const probe = await p.post({ model: "opencode-route/t-a-only", messages: MSGS });
    assert.equal(probe.status, 200);
    assert.equal((await probe.json()).choices[0].message.content, "LEAF-A");
    const primaryAgain = await p.post({ model: "opencode-route/t-ordered", messages: MSGS });
    assert.equal(primaryAgain.status, 200);
    assert.equal(
      (await primaryAgain.json()).choices[0].message.content,
      "LEAF-A",
    );
  } finally {
    await p.cleanup();
  }
});

test("RateLimit-only failures still demote a leaf until its next success", async () => {
  const p = await startProxy();
  try {
    const db = new DatabaseSync(p.dbPath);
    const insert = db.prepare(
      `INSERT INTO route_attempt
       (time_started, time_ended, route_id, provider_id, model_id, outcome, error_tag, output_started)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
    );
    const now = Date.now();
    for (let i = 0; i < 3; i++) {
      insert.run(
        now - 1000 + i,
        now - 999 + i,
        "t-ordered",
        "leafA",
        "m-a",
        "failure",
        "RateLimit",
        0,
      );
    }
    db.close();

    const fallback = await p.post({ model: "opencode-route/t-ordered", messages: MSGS });
    assert.equal(fallback.status, 200);
    assert.equal((await fallback.json()).choices[0].message.content, "LEAF-B");
    assert.equal(
      p.attempts().filter((row) => row.provider_id === "leafA").length,
      3,
    );

    p.leafA.mode = "ok";
    const recovered = await p.post({ model: "opencode-route/t-a-only", messages: MSGS });
    assert.equal(recovered.status, 200);
    const primaryAgain = await p.post({ model: "opencode-route/t-ordered", messages: MSGS });
    assert.equal(primaryAgain.status, 200);
    assert.equal(
      (await primaryAgain.json()).choices[0].message.content,
      "LEAF-A",
    );
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
    const rows = await waitForAttempts(p, 1);
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

test("first-token timing includes delay past the pre-content window", async () => {
  const p = await startProxy();
  try {
    p.leafA.mode = "delayed-stream";
    const res = await p.post({
      model: "opencode-route/t-ordered",
      messages: MSGS,
      stream: true,
    });
    assert.equal(res.status, 200);
    assert.ok((await res.text()).includes("LEAF-A"));
    const rows = await waitForAttempts(p, 1);
    assert.equal(rows.length, 1);
    assert.equal(rows[0].outcome, "success");
    assert.ok(rows[0].first_token_ms > 200, `first_token_ms=${rows[0].first_token_ms}`);
  } finally {
    await p.cleanup();
  }
});

test("pre-content SSE quota error fails over to the next stream", async () => {
  const p = await startProxy();
  try {
    p.leafA.mode = "sse-rate";
    const res = await p.post({
      model: "opencode-route/t-ordered",
      messages: MSGS,
      stream: true,
    });
    assert.equal(res.status, 200);
    const text = await res.text();
    assert.ok(text.includes("LEAF-B"), text);
    assert.ok(!text.includes("quota exhausted"), text);
    const rows = await waitForAttempts(p, 2);
    assert.equal(rows.length, 2);
    assert.equal(rows[0].outcome, "failure");
    assert.equal(rows[0].error_tag, "RateLimit");
    assert.equal(rows[0].error_status, 429);
    assert.equal(rows[0].failed_over_to, "leafB/m-b");
    assert.equal(rows[0].output_started, 0);
    assert.equal(JSON.parse(rows[0].quota).retryAfterMs, 30_000);
    assert.equal(rows[1].outcome, "success");
    assert.equal(rows[1].provider_id, "leafB");
    assert.equal(rows[1].output_started, 1);
  } finally {
    await p.cleanup();
  }
});

test("pre-content SSE error after the buffer window still fails over cleanly", async () => {
  const p = await startProxy();
  try {
    p.leafA.mode = "delayed-sse-rate";
    const res = await p.post({
      model: "opencode-route/t-ordered",
      messages: MSGS,
      stream: true,
    });
    assert.equal(res.status, 200);
    const text = await res.text();
    assert.ok(text.includes("LEAF-B"), text);
    assert.ok(!text.includes("quota exhausted"), text);
    assert.ok(!text.includes("heartbeat"), text);
    const rows = await waitForAttempts(p, 2);
    assert.equal(rows.length, 2);
    assert.equal(rows[0].outcome, "failure");
    assert.equal(rows[0].error_tag, "RateLimit");
    assert.equal(rows[0].failed_over_to, "leafB/m-b");
    assert.equal(rows[0].output_started, 0);
    assert.equal(JSON.parse(rows[0].quota).retryAfterMs, 30_000);
    assert.equal(rows[1].outcome, "success");
  } finally {
    await p.cleanup();
  }
});

test("pre-content transport break fails over without committing the heartbeat", async () => {
  const p = await startProxy();
  try {
    p.leafA.mode = "stream-break";
    const res = await p.post({
      model: "opencode-route/t-ordered",
      messages: MSGS,
      stream: true,
    });
    assert.equal(res.status, 200);
    const text = await res.text();
    assert.ok(text.includes("LEAF-B"), text);
    assert.ok(!text.includes("heartbeat"), text);
    const rows = await waitForAttempts(p, 2);
    assert.equal(rows.length, 2);
    assert.equal(rows[0].outcome, "failure");
    assert.equal(rows[0].error_tag, "Transport");
    assert.equal(rows[0].failed_over_to, "leafB/m-b");
    assert.equal(rows[0].output_started, 0);
    assert.equal(rows[1].outcome, "success");
  } finally {
    await p.cleanup();
  }
});

test("stream errors after content do not fail over", async () => {
  const p = await startProxy();
  try {
    p.leafA.mode = "late-sse-rate";
    const res = await p.post({
      model: "opencode-route/t-ordered",
      messages: MSGS,
      stream: true,
    });
    assert.equal(res.status, 200);
    const text = await res.text();
    assert.ok(text.includes("LEAF-A"), text);
    assert.ok(text.includes("late quota exhausted"), text);
    assert.equal(p.leafB.hits, 0);
    const rows = await waitForAttempts(p, 1);
    assert.equal(rows.length, 1);
    assert.equal(rows[0].outcome, "failure");
    assert.equal(rows[0].output_started, 1);
  } finally {
    await p.cleanup();
  }
});

test("a first content event larger than the preface window is relayed, not rejected (#5558)", async () => {
  const p = await startProxy();
  try {
    p.leafA.mode = "big-first-event";
    const res = await p.post({
      model: "opencode-route/t-ordered",
      messages: MSGS,
      stream: true,
    });
    assert.equal(res.status, 200);
    const text = await res.text();
    assert.ok(text.includes("A".repeat(1024)), "large first event must reach the client");
    assert.ok(text.includes("[DONE]"));
    assert.equal(p.leafB.hits, 0);
    const rows = await waitForAttempts(p, 1);
    assert.equal(rows.length, 1);
    assert.equal(rows[0].outcome, "success");
    assert.equal(rows[0].output_started, 1);
  } finally {
    await p.cleanup();
  }
});

test("pre-content preface overflow fails over and cancels the rejected stream (#5558)", async () => {
  const p = await startProxy();
  try {
    p.leafA.mode = "preface-flood";
    const res = await p.post({
      model: "opencode-route/t-ordered",
      messages: MSGS,
      stream: true,
    });
    assert.equal(res.status, 200);
    const text = await res.text();
    assert.ok(text.includes("LEAF-B"), text);
    const rows = await waitForAttempts(p, 2);
    assert.equal(rows.length, 2);
    assert.equal(rows[0].outcome, "failure");
    assert.equal(rows[0].output_started, 0);
    assert.equal(rows[1].outcome, "success");
    // The rejected leaf's stream was cancelled, not left generating.
    for (let i = 0; i < 50 && !p.leafA.prefaceAborted; i++)
      await new Promise((r) => setTimeout(r, 100));
    assert.equal(p.leafA.prefaceAborted, true);
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
    for (const provider of ["constructor", "__proto__", "toString", "hasOwnProperty", "valueOf"]) {
      const proto = await p.post({ model: `${provider}/m-b`, messages: MSGS });
      assert.equal(proto.status, 502, `prototype spelling ${provider} must miss provider tables`);
      assert.match((await proto.json()).error.message, /unknown|no credential|provider/i);
    }
    const bad = await p.post({ model: "opencode-route/t-ordered", messages: "x" });
    assert.equal(bad.status, 400);
  } finally {
    await p.cleanup();
  }
});

test("chunk boundary inside a multibyte character arrives intact (#4335)", async () => {
  const p = await startProxy();
  try {
    const text = "café ☃ fin";
    const payload = JSON.stringify({ model: "opencode-route/t-ordered", messages: [{ role: "user", content: text }] });
    const bytes = Buffer.from(payload, "utf8");
    // Split inside the é bytes (0xC3 0xA9) and inside the ☃ bytes
    // (0xE2 0x98 0x83): per-chunk string coercion would emit U+FFFD here.
    const eAcute = bytes.indexOf(Buffer.from("é", "utf8"));
    const snowman = bytes.indexOf(Buffer.from("☃", "utf8"));
    assert.ok(eAcute > 0 && snowman > eAcute);
    const parts = [bytes.subarray(0, eAcute + 1), bytes.subarray(eAcute + 1, snowman + 2), bytes.subarray(snowman + 2)];
    const res = await fetch(`${p.base}/v1/chat/completions`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: new ReadableStream({
        start(c) {
          for (const part of parts) c.enqueue(part);
          c.close();
        },
      }),
      duplex: "half",
    });
    assert.equal(res.status, 200);
    assert.equal(p.leafA.last.body.messages[0].content, text);
  } finally {
    await p.cleanup();
  }
});

test("oversize body is rejected with 413 (#4335)", async () => {
  const p = await startProxy();
  try {
    const big = "x".repeat(51 * 1024 * 1024);
    const res = await p.post({ model: "opencode-route/t-ordered", messages: [{ role: "user", content: big }] });
    assert.equal(res.status, 413);
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

test("provider routes skip prototype-spelling members like other unknown providers", async () => {
  const p = await startProxy();
  try {
    const res = await p.post({ model: "opencode-route/t-group-prototype-spellings", messages: MSGS });
    assert.equal(res.status, 200);
    assert.equal((await res.json()).choices[0].message.content, "LEAF-A");
    assert.deepEqual(p.attempts().map(row => row.provider_id), ["leafA"]);
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
    // #11176: an unknown timezone and a malformed daysOfWeek also never
    // match. A string daysOfWeek used to widen to every day, and an
    // unknown name beside a real one (Wed 12:00 here) used to match.
    ["t-window-bad-tz", AT_1200],
    ["t-window-bad-days-string", AT_1200],
    ["t-window-bad-days-name", AT_1200],
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

test("full weekday names still match their day (#11176)", async () => {
  // 2026-11-04 is a Wednesday; ["Wednesday"] normalizes to "wed".
  const p = await startProxy({ nowMs: Date.UTC(2026, 10, 4, 12, 0) });
  try {
    const res = await p.post({ model: "opencode-route/t-window-full-day-names", messages: MSGS });
    assert.equal((await res.json()).choices[0].message.content, "LEAF-A");
  } finally {
    await p.cleanup();
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
    // #11176: a bad timezone or daysOfWeek is named with its value too.
    assert.ok(log.includes("bad-tz") && log.includes("America/Chicagoo"), `log names a bad timezone:\n${log}`);
    assert.ok(log.includes("bad-days-string") && log.includes('daysOfWeek="fri"'), `log names a string daysOfWeek:\n${log}`);
    assert.ok(log.includes("bad-days-name") && log.includes("funday"), `log names a bad weekday:\n${log}`);
    assert.ok(!log.includes('model_route_windows["full-day-names"]'), `full weekday names are valid:\n${log}`);
  } finally {
    await p.cleanup();
  }
});

// ------------------------------------------------- #9378 stage 2: window context on route_attempt

// Read the newest attempt's recorded window context.
const latestWindowRow = (dbPath) => {
  const db = new DatabaseSync(dbPath);
  const row = db
    .prepare(
      "SELECT provider_id, window_names, window_class, holiday, priority_band FROM route_attempt ORDER BY id DESC LIMIT 1",
    )
    .get();
  db.close();
  return { ...row };
};

test("route_attempt carries window context, flipping with the window switch (#9378)", async () => {
  // 2026-11-04 is a Wednesday. 02:30 UTC is inside the peak window;
  // 05:00 UTC is off-peak for the bound route (peak ended at 04:00).
  const p = await startProxy({ nowMs: Date.UTC(2026, 10, 4, 2, 30) });
  try {
    const peak = await p.post({ model: "opencode-route/t-window-bound", messages: MSGS });
    assert.equal(peak.status, 200);
    assert.equal((await peak.json()).choices[0].message.content, "LEAF-A");
    assert.deepEqual(latestWindowRow(p.dbPath), {
      provider_id: "leafA",
      window_names: "peak",
      window_class: "peak",
      holiday: "unknown",
      priority_band: "interactive",
    });
  } finally {
    await p.cleanup();
  }
  const q = await startProxy({ nowMs: Date.UTC(2026, 10, 4, 5, 0) });
  try {
    const off = await q.post({ model: "opencode-route/t-window-bound", messages: MSGS });
    assert.equal(off.status, 200);
    assert.equal((await off.json()).choices[0].message.content, "LEAF-B");
    assert.deepEqual(latestWindowRow(q.dbPath), {
      provider_id: "leafB",
      window_names: "",
      window_class: "neutral",
      holiday: "unknown",
      priority_band: "interactive",
    });
  } finally {
    await q.cleanup();
  }
});

test("unbound routes see every window; peak outranks preburn (#9378)", async () => {
  const p = await startProxy({ nowMs: Date.UTC(2026, 10, 4, 2, 30) });
  try {
    const res = await p.post({ model: "opencode-route/t-window", messages: MSGS });
    assert.equal(res.status, 200);
    assert.equal((await res.json()).choices[0].message.content, "LEAF-A");
    // Unbound: every defined window counts. At Wed 02:30 UTC the
    // full-day fixture window and the preburn overlap join peak;
    // absent classes record as the peak default.
    assert.deepEqual(latestWindowRow(p.dbPath), {
      provider_id: "leafA",
      window_names: "peak,full-day-names,preburn-overlap",
      window_class: "peak",
      holiday: "unknown",
      priority_band: "interactive",
    });
  } finally {
    await p.cleanup();
  }
});

test("off-peak class records on its own window (#9378)", async () => {
  const p = await startProxy({ nowMs: Date.UTC(2026, 10, 4, 5, 0) });
  try {
    const res = await p.post({ model: "opencode-route/t-window-bound-multi", messages: MSGS });
    assert.equal(res.status, 200);
    assert.equal((await res.json()).choices[0].message.content, "LEAF-A");
    assert.deepEqual(latestWindowRow(p.dbPath), {
      provider_id: "leafA",
      window_names: "offpeak-x",
      window_class: "off-peak",
      holiday: "unknown",
      priority_band: "interactive",
    });
  } finally {
    await p.cleanup();
  }
});

test("unknown window class warns but still gates (#9378)", async () => {
  // 2026-11-02 is a Monday; badclass-x covers the whole day.
  const p = await startProxy({ nowMs: Date.UTC(2026, 10, 2, 12, 0) });
  try {
    const res = await p.post({ model: "opencode-route/t-window-badclass", messages: MSGS });
    assert.equal(res.status, 200);
    assert.equal((await res.json()).choices[0].message.content, "LEAF-A");
    let log = "";
    for (let i = 0; i < 20; i++) {
      log = p.log();
      if (log.includes("badclass-x") && log.includes("unknown class")) break;
      await new Promise((r) => setTimeout(r, 100));
    }
    assert.ok(
      log.includes("badclass-x") && log.includes("unknown class"),
      `log names the bad class:\n${log}`,
    );
    // The recording-label typo records as the peak default; the gate
    // still follows the window's time rule.
    assert.deepEqual(latestWindowRow(p.dbPath), {
      provider_id: "leafA",
      window_names: "badclass-x",
      window_class: "peak",
      holiday: "unknown",
      priority_band: "interactive",
    });
  } finally {
    await p.cleanup();
  }
});

test("a walled leg stays walled across a window switch (#9378 G2)", async () => {
  const now = Date.UTC(2026, 10, 4, 2, 30); // Wednesday, inside peak
  const p = await startProxy({ nowMs: now });
  try {
    seedQuotaFailure(p.dbPath, {
      provider: "leafA",
      model: "m-a",
      atMs: now - 60_000,
      retryAfterMs: 3_600_000,
    });
    const res = await p.post({ model: "opencode-route/t-window-bound", messages: MSGS });
    assert.equal(res.status, 200);
    assert.equal((await res.json()).choices[0].message.content, "LEAF-B");
    assert.equal(p.leafA.hits, 0, "the walled peak lead must not be attempted");
    // The failover attempt still carries the peak switch state.
    assert.deepEqual(latestWindowRow(p.dbPath), {
      provider_id: "leafB",
      window_names: "peak",
      window_class: "peak",
      holiday: "unknown",
      priority_band: "interactive",
    });
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

// Seed one quota-class failure carrying NO parseable reset (a
// reset-less 429 shape, or a 402 paymentRequired record), as the
// inferred-wall fixtures need (#13693). Written straight into the
// fixture ledger the proxy reads.
const seedQuotaClassFailure = (
  dbPath,
  { provider, model, atMs, endedMs, quota, status = 429 },
) => {
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
    status,
    "leaf HTTP 429: quota",
    1,
    0,
    null,
    JSON.stringify(quota),
  );
  db.close();
};

// Seed one authentication failure for a (provider, model) pair, as if
// the leaf had answered 403 (or 401) at `atMs`. Written straight into
// the fixture ledger the proxy reads.
const seedAuthFailure = (
  dbPath,
  { provider, model, atMs, endedMs, status = 403 },
) => {
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
    "Authentication",
    null,
    status,
    "leaf HTTP 403: disabled",
    0,
    0,
    null,
    null,
  );
  db.close();
};

// Seed one success for a (provider, model) pair completing at
// `endedMs` (started `startedMs`), as if the leaf had served.
const seedSuccess = (dbPath, { provider, model, startedMs, endedMs }) => {
  const db = new DatabaseSync(dbPath);
  db.prepare(
    `INSERT INTO route_attempt (
       time_started, time_ended, session_id, assistant_message_id,
       route_id, provider_id, model_id, variant, outcome,
       error_tag, error_code, error_status, error_message, retryable,
       output_started, failed_over_to, quota
     ) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)`,
  ).run(
    startedMs,
    endedMs,
    null,
    null,
    "seed",
    provider,
    model,
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

// ------------------------------------------------------------- #12228 auth bench

test("a 403-auth-failed pair is excluded while another candidate is live (#12228)", async () => {
  const p = await startProxy({ nowMs: GATE_NOW });
  try {
    seedAuthFailure(p.dbPath, {
      provider: "leafA",
      model: "m-a",
      atMs: GATE_NOW - 60_000,
    });
    const res = await p.post({ model: "opencode-route/t-ordered", messages: MSGS });
    assert.equal(res.status, 200);
    assert.equal((await res.json()).choices[0].message.content, "LEAF-B");
    assert.equal(p.leafA.hits, 0, "dead leaf must not be attempted");
    assert.ok(
      p.log().includes("auth bench excluded leafA/m-a"),
      `bench decision should be in the proxy diagnostics: ${p.log()}`,
    );
  } finally {
    await p.cleanup();
  }
});

test("an auth-bench pair returns after a successful probe (#12228)", async () => {
  const p = await startProxy({ nowMs: GATE_NOW });
  try {
    seedAuthFailure(p.dbPath, {
      provider: "leafA",
      model: "m-a",
      atMs: GATE_NOW - 60_000,
    });
    seedSuccess(p.dbPath, {
      provider: "leafA",
      model: "m-a",
      startedMs: GATE_NOW - 30_000,
      endedMs: GATE_NOW - 20_000,
    });
    const res = await p.post({ model: "opencode-route/t-ordered", messages: MSGS });
    assert.equal(res.status, 200);
    assert.equal((await res.json()).choices[0].message.content, "LEAF-A");
    assert.equal(p.leafA.hits, 1, "successful probe re-admits the leaf");
  } finally {
    await p.cleanup();
  }
});

test("when every candidate is auth-benched, the route still walks in order (#12228)", async () => {
  const p = await startProxy({ nowMs: GATE_NOW });
  try {
    seedAuthFailure(p.dbPath, {
      provider: "leafA",
      model: "m-a",
      atMs: GATE_NOW - 60_000,
    });
    seedAuthFailure(p.dbPath, {
      provider: "leafB",
      model: "m-b",
      atMs: GATE_NOW - 60_000,
    });
    // A benched route is still usable as a last resort, just like the
    // quota gate: preserve availability rather than return no route.
    const res = await p.post({ model: "opencode-route/t-ordered", messages: MSGS });
    assert.equal(res.status, 200);
    assert.equal((await res.json()).choices[0].message.content, "LEAF-A");
    assert.ok(
      p.log().includes("auth bench gate bypassed"),
      `all-benched fallback should be explicit: ${p.log()}`,
    );
  } finally {
    await p.cleanup();
  }
});

// ------------------------------------------------------------ #13693 walls

test("429 with an HTTP-date Retry-After gates the pair until that date (#13693)", async () => {
  // Oracle: #13693 RC-1(a) — an HTTP-date Retry-After is a reset
  // shape; the pair must gate until that date, not record
  // retryAfterMs: null and stay walkable.
  const p = await startProxy();
  try {
    p.leafA.mode = "rate-http-date";
    const first = await p.post({ model: "opencode-route/t-ordered", messages: MSGS });
    assert.equal(first.status, 200);
    assert.equal((await first.json()).choices[0].message.content, "LEAF-B");
    const rows = await waitForAttempts(p, 2);
    const quota = JSON.parse(rows[0].quota);
    assert.ok(
      Number.isFinite(quota.retryAfterMs) &&
        quota.retryAfterMs > 0 &&
        quota.retryAfterMs <= 3600_000,
      `HTTP-date Retry-After not recorded as a positive delta: ${rows[0].quota}`,
    );
    const second = await p.post({ model: "opencode-route/t-ordered", messages: MSGS });
    assert.equal(second.status, 200);
    assert.equal((await second.json()).choices[0].message.content, "LEAF-B");
    assert.equal(p.leafA.hits, 1, "the date-gated leaf must not be re-attempted");
  } finally {
    await p.cleanup();
  }
});

test("429 with duration-prose body gates the pair (#13693)", async () => {
  // Oracle: #13693 RC-1(a) — "try again in 20 minutes" in the body,
  // with no header, is a reset shape (receipt + duration).
  const p = await startProxy();
  try {
    p.leafA.mode = "rate-prose";
    const first = await p.post({ model: "opencode-route/t-ordered", messages: MSGS });
    assert.equal(first.status, 200);
    assert.equal((await first.json()).choices[0].message.content, "LEAF-B");
    const rows = await waitForAttempts(p, 2);
    const quota = JSON.parse(rows[0].quota);
    const resetMs = Date.parse(quota.resetsAt ?? quota.resetAt ?? NaN);
    assert.ok(
      Number.isFinite(resetMs) && resetMs > Date.now() + 10 * 60_000,
      `duration prose not recorded as a ~20-minute reset: ${rows[0].quota}`,
    );
    const second = await p.post({ model: "opencode-route/t-ordered", messages: MSGS });
    assert.equal(second.status, 200);
    assert.equal((await second.json()).choices[0].message.content, "LEAF-B");
    assert.equal(p.leafA.hits, 1);
  } finally {
    await p.cleanup();
  }
});

test("429 with {\"retry_after\": 30} JSON gates the pair (#13693)", async () => {
  // Oracle: #13693 RC-1(a) — a small number under an after key is
  // seconds-from-receipt, not an epoch (which parsed as NaN before).
  const p = await startProxy();
  try {
    p.leafA.mode = "rate-json-retry-after";
    const first = await p.post({ model: "opencode-route/t-ordered", messages: MSGS });
    assert.equal(first.status, 200);
    assert.equal((await first.json()).choices[0].message.content, "LEAF-B");
    const rows = await waitForAttempts(p, 2);
    const quota = JSON.parse(rows[0].quota);
    const resetMs = Date.parse(quota.resetsAt ?? quota.resetAt ?? NaN);
    assert.ok(
      Number.isFinite(resetMs) &&
        resetMs > Date.now() &&
        resetMs <= Date.now() + 60_000,
      `retry_after seconds not recorded as a near reset: ${rows[0].quota}`,
    );
    const second = await p.post({ model: "opencode-route/t-ordered", messages: MSGS });
    assert.equal(second.status, 200);
    assert.equal((await second.json()).choices[0].message.content, "LEAF-B");
    assert.equal(p.leafA.hits, 1);
  } finally {
    await p.cleanup();
  }
});

test("429 with an x-ratelimit-reset header gates the pair (#13693)", async () => {
  // Oracle: #13693 RC-1(d) — x-ratelimit-reset (epoch seconds here)
  // is recorded as resetsAtMs in the quota detail and gates.
  const p = await startProxy();
  try {
    p.leafA.mode = "rate-x-reset";
    const first = await p.post({ model: "opencode-route/t-ordered", messages: MSGS });
    assert.equal(first.status, 200);
    assert.equal((await first.json()).choices[0].message.content, "LEAF-B");
    const rows = await waitForAttempts(p, 2);
    const quota = JSON.parse(rows[0].quota);
    assert.ok(
      Number.isFinite(quota.resetsAtMs) &&
        quota.resetsAtMs > Date.now() + 30 * 60_000,
      `x-ratelimit-reset not recorded as resetsAtMs: ${rows[0].quota}`,
    );
    const second = await p.post({ model: "opencode-route/t-ordered", messages: MSGS });
    assert.equal(second.status, 200);
    assert.equal((await second.json()).choices[0].message.content, "LEAF-B");
    assert.equal(p.leafA.hits, 1);
  } finally {
    await p.cleanup();
  }
});

test("a reset-less failure after a reset-carrying one keeps the gate (#13693)", async () => {
  // Oracle: #13693 RC-2 — the gate judges the pair's current failure
  // RUN: a later reset-less quota failure must not dissolve the
  // wall the earlier failure recorded.
  const p = await startProxy({ nowMs: GATE_NOW });
  try {
    seedQuotaFailure(p.dbPath, {
      provider: "leafA",
      model: "m-a",
      atMs: GATE_NOW - 120_000,
      retryAfterMs: 3_600_000,
    });
    seedQuotaClassFailure(p.dbPath, {
      provider: "leafA",
      model: "m-a",
      atMs: GATE_NOW - 60_000,
      quota: { status: 429 },
    });
    const res = await p.post({ model: "opencode-route/t-ordered", messages: MSGS });
    assert.equal(res.status, 200);
    assert.equal((await res.json()).choices[0].message.content, "LEAF-B");
    assert.equal(p.leafA.hits, 0, "the run's recorded reset must still gate");
    assert.ok(
      p.log().includes("known-exhausted gate excluded leafA/m-a"),
      `run gate decision should be in the proxy diagnostics: ${p.log()}`,
    );
  } finally {
    await p.cleanup();
  }
});

test("three reset-less quota failures infer a bounded wall (#13693)", async () => {
  // Oracle: #13693 RC-1(c) — >=3 quota-class failures in the
  // demotion window with no success since the run began exclude
  // the pair while a live pair exists, even with no stated reset.
  const p = await startProxy({ nowMs: GATE_NOW });
  try {
    for (const atMs of [GATE_NOW - 180_000, GATE_NOW - 120_000, GATE_NOW - 60_000])
      seedQuotaClassFailure(p.dbPath, {
        provider: "leafA",
        model: "m-a",
        atMs,
        quota: { status: 429 },
      });
    const res = await p.post({ model: "opencode-route/t-ordered", messages: MSGS });
    assert.equal(res.status, 200);
    assert.equal((await res.json()).choices[0].message.content, "LEAF-B");
    assert.equal(p.leafA.hits, 0, "the inferred-walled leaf must not be attempted");
    assert.ok(
      p.log().includes("inferred wall excluded leafA/m-a"),
      `inferred exclusion should be in the proxy diagnostics: ${p.log()}`,
    );
  } finally {
    await p.cleanup();
  }
});

test("an inferred wall expires and the pair is seated again (#13693)", async () => {
  // Oracle: #13693 RC-1(c) — the inferred wall is BOUNDED (15 min
  // from the run's latest failure for a first inference): once it
  // passes, the pair is no longer excluded; the next seating is
  // the probe. (Demotion may still reorder the pair — this test
  // oracles the gate's own decision, recorded in the diagnostics.)
  const p = await startProxy({ nowMs: GATE_NOW + 16 * 60_000 });
  try {
    for (const atMs of [GATE_NOW - 180_000, GATE_NOW - 120_000, GATE_NOW - 60_000])
      seedQuotaClassFailure(p.dbPath, {
        provider: "leafA",
        model: "m-a",
        atMs,
        quota: { status: 429 },
      });
    const res = await p.post({ model: "opencode-route/t-ordered", messages: MSGS });
    assert.equal(res.status, 200);
    await res.json();
    assert.ok(
      !p.log().includes("inferred wall excluded leafA/m-a"),
      `expired inferred wall must not exclude: ${p.log()}`,
    );
  } finally {
    await p.cleanup();
  }
});

test("a 402-only run also infers a wall (balance wall) (#13693)", async () => {
  // Oracle: #13693 RC-1(c) — 402 paymentRequired records are
  // quota-class failures for the inferred wall.
  const p = await startProxy({ nowMs: GATE_NOW });
  try {
    for (const atMs of [GATE_NOW - 180_000, GATE_NOW - 120_000, GATE_NOW - 60_000])
      seedQuotaClassFailure(p.dbPath, {
        provider: "leafA",
        model: "m-a",
        atMs,
        quota: { paymentRequired: true },
        status: 402,
      });
    const res = await p.post({ model: "opencode-route/t-ordered", messages: MSGS });
    assert.equal(res.status, 200);
    assert.equal((await res.json()).choices[0].message.content, "LEAF-B");
    assert.equal(p.leafA.hits, 0);
    assert.ok(
      p.log().includes("inferred wall excluded leafA/m-a"),
      `402 inference should be in the proxy diagnostics: ${p.log()}`,
    );
  } finally {
    await p.cleanup();
  }
});

test("all candidates reset-gated with a recent attempt: 503 + Retry-After (#13693)", async () => {
  // Oracle: #13693 RC-4 — when every candidate is gated on a known
  // future reset >60s away and a walk already paid attempts within
  // the last 60s, the proxy paces with 503 + Retry-After instead of
  // walking into the wall again.
  const p = await startProxy({ nowMs: GATE_NOW });
  try {
    seedQuotaFailure(p.dbPath, {
      provider: "leafA",
      model: "m-a",
      atMs: GATE_NOW - 30_000,
      retryAfterMs: 3_600_000,
    });
    seedQuotaFailure(p.dbPath, {
      provider: "leafB",
      model: "m-b",
      atMs: GATE_NOW - 30_000,
      retryAfterMs: 3_600_000,
    });
    const res = await p.post({ model: "opencode-route/t-ordered", messages: MSGS });
    assert.equal(res.status, 503);
    const retryAfter = Number(res.headers.get("retry-after"));
    assert.ok(
      Number.isFinite(retryAfter) && retryAfter > 3500 && retryAfter <= 3600,
      `Retry-After should run to the earliest reset: ${res.headers.get("retry-after")}`,
    );
    const body = await res.json();
    assert.match(body.error.message, /quota-walled/);
    assert.equal(p.leafA.hits + p.leafB.hits, 0, "a paced request must not walk");
    assert.ok(
      p.log().includes("wall pacing"),
      `pacing decision should be in the proxy diagnostics: ${p.log()}`,
    );
  } finally {
    await p.cleanup();
  }
});

test("all candidates inferred-gated: route still walks, fail-safe preserved (#13693)", async () => {
  // Oracle: #13693 RC-4 — pacing applies ONLY to reset-carrying
  // known-exhausted exclusions; soft (inferred) gates keep the
  // walk-anyway fail-safe byte-for-byte.
  const p = await startProxy({ nowMs: GATE_NOW });
  try {
    for (const atMs of [GATE_NOW - 180_000, GATE_NOW - 120_000, GATE_NOW - 60_000]) {
      seedQuotaClassFailure(p.dbPath, {
        provider: "leafA",
        model: "m-a",
        atMs,
        quota: { status: 429 },
      });
      seedQuotaClassFailure(p.dbPath, {
        provider: "leafB",
        model: "m-b",
        atMs,
        quota: { status: 429 },
      });
    }
    const res = await p.post({ model: "opencode-route/t-ordered", messages: MSGS });
    assert.equal(res.status, 200);
    assert.equal((await res.json()).choices[0].message.content, "LEAF-A");
    assert.ok(
      p.log().includes("gate bypassed"),
      `inferred bypass should be in the proxy diagnostics: ${p.log()}`,
    );
  } finally {
    await p.cleanup();
  }
});

test("429 with {\"retryDelay\": \"30s\"} JSON gates the pair (#13693)", async () => {
  // Oracle: #13693 RC-1(a) — a pure-duration string under a retry
  // key (Google-style retryDelay) is a duration from receipt, not
  // an instant to Date.parse.
  const p = await startProxy();
  try {
    p.leafA.mode = "rate-json-retry-delay";
    const first = await p.post({ model: "opencode-route/t-ordered", messages: MSGS });
    assert.equal(first.status, 200);
    assert.equal((await first.json()).choices[0].message.content, "LEAF-B");
    const rows = await waitForAttempts(p, 2);
    const quota = JSON.parse(rows[0].quota);
    const resetMs = Date.parse(quota.resetsAt ?? quota.resetAt ?? NaN);
    assert.ok(
      Number.isFinite(resetMs) &&
        resetMs > Date.now() &&
        resetMs <= Date.now() + 60_000,
      `retryDelay duration not recorded as a near reset: ${rows[0].quota}`,
    );
    const second = await p.post({ model: "opencode-route/t-ordered", messages: MSGS });
    assert.equal(second.status, 200);
    assert.equal((await second.json()).choices[0].message.content, "LEAF-B");
    assert.equal(p.leafA.hits, 1);
  } finally {
    await p.cleanup();
  }
});

// ---------------- Part 2: purpose-routed endpoints (#13675 P2) ----------------
//
// Helpers for the non-chat doors. p.post only targets chat; these hit
// the purpose endpoints directly.
const postPurpose = (p, endpoint, payload, raw) =>
  fetch(`${p.base}${endpoint}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: raw !== undefined ? raw : JSON.stringify(payload),
  });

const postTranscription = (p, model, fileBytes = "fake-audio-bytes") => {
  const boundary = "----part2testboundary";
  const body =
    `--${boundary}\r\nContent-Disposition: form-data; name="model"\r\n\r\n${model}\r\n` +
    `--${boundary}\r\nContent-Disposition: form-data; name="file"; filename="a.wav"\r\n` +
    `Content-Type: audio/wav\r\n\r\n${fileBytes}\r\n` +
    `--${boundary}--\r\n`;
  return fetch(`${p.base}/v1/audio/transcriptions`, {
    method: "POST",
    headers: { "content-type": `multipart/form-data; boundary=${boundary}` },
    body,
  });
};

test("decisions have no OpenAI default: plain leaves refuse with 400 (#13675 P2)", async () => {
  const p = await startProxy();
  try {
    const res = await postPurpose(p, "/v1/decisions", {
      model: "opencode-route/t-decide",
      input: { state: 1 },
      questions: { ok: { type: "noul" } },
    });
    assert.equal(res.status, 400);
    assert.match(await res.text(), /no members serving decision/);
    assert.equal(p.leafA.hits, 0);
    assert.equal(p.leafB.hits, 0);
  } finally {
    await p.cleanup();
  }
});

test("decisions dispatch natively via google-decisions with x-goog-api-key (#13675 P2)", async () => {
  const p = await startProxy();
  try {
    const res = await postPurpose(p, "/v1/decisions", {
      model: "opencode-route/t-decide-goog",
      input: { state: 1 },
      questions: { ok: { type: "noul" } },
    });
    assert.equal(res.status, 200);
    const json = await res.json();
    assert.equal(json.decision, "yes");
    assert.ok(p.leafA.last.url.includes("/decisions"));
    assert.equal(p.leafA.last.headers["x-goog-api-key"], "key-a");
    assert.ok(!("authorization" in p.leafA.last.headers));
    const rows = await waitForAttempts(p, 1);
    assert.equal(rows[0].route_id, "t-decide-goog");
    assert.equal(rows[0].outcome, "success");
    assert.equal(rows[0].session_id, null);
  } finally {
    await p.cleanup();
  }
});

test("decisions dispatch via jev-systemone with Bearer auth (#13675 P2)", async () => {
  const p = await startProxy();
  try {
    const res = await postPurpose(p, "/v1/decisions", {
      model: "opencode-route/t-decide-zen",
      input: { state: 2 },
      questions: { ok: { type: "noul" } },
    });
    assert.equal(res.status, 200);
    assert.equal((await res.json()).decision, "yes");
    assert.ok(p.leafA.last.url.endsWith("/decisions"));
    assert.equal(p.leafA.last.headers.authorization, "Bearer key-a");
  } finally {
    await p.cleanup();
  }
});

test("decisions dispatch via fireworks passthrough with required path (#13675 P2)", async () => {
  const p = await startProxy();
  try {
    const res = await postPurpose(p, "/v1/decisions", {
      model: "opencode-route/t-decide-fw",
      input: { state: 3 },
      questions: { ok: { type: "noul" } },
    });
    assert.equal(res.status, 200);
    assert.equal((await res.json()).model, "m-fd");
  } finally {
    await p.cleanup();
  }
});

test("unknown decision kinds never invent a surface (#13675 P2)", async () => {
  const p = await startProxy();
  try {
    const res = await postPurpose(p, "/v1/decisions", {
      model: "opencode-route/t-decide-weird",
      input: { state: 1 },
      questions: { ok: { type: "noul" } },
    });
    assert.equal(res.status, 400);
    assert.match(await res.text(), /no members serving decision/);
    assert.equal(p.leafA.hits, 0);
  } finally {
    await p.cleanup();
  }
});

test("new-endpoint unknown routes 404; pool-purpose mismatches refuse 400 (#13675 P2)", async () => {
  const p = await startProxy();
  try {
    const unknown = await postPurpose(p, "/v1/decisions", {
      model: "opencode-route/no-such-pool",
      input: { state: 1 },
      questions: { ok: { type: "noul" } },
    });
    assert.equal(unknown.status, 404);
    assert.match(await unknown.text(), /unknown route/);
    const mismatch = await postPurpose(p, "/v1/embeddings", {
      model: "opencode-route/t-decide-goog",
      input: "hello",
    });
    assert.equal(mismatch.status, 400);
    assert.match(await mismatch.text(), /cannot serve embed/);
    assert.equal(p.leafA.hits, 0);
    assert.equal(p.leafB.hits, 0);
  } finally {
    await p.cleanup();
  }
});

test("chat calls naming a decision pool are refused, never forwarded (#13675 P2)", async () => {
  const p = await startProxy();
  try {
    const res = await p.post({
      model: "opencode-route/t-decide-goog",
      messages: MSGS,
    });
    assert.equal(res.status, 502);
    assert.match(await res.text(), /cannot serve chat/);
    assert.equal(p.leafA.hits, 0);
    assert.equal(p.leafB.hits, 0);
  } finally {
    await p.cleanup();
  }
});

test("runtime designations filter leaves at dispatch (#13675 P2)", async () => {
  const p = await startProxy();
  try {
    const res = await postPurpose(p, "/v1/decisions", {
      model: "opencode-route/t-decide-desig",
      input: { state: 1 },
      questions: { ok: { type: "noul" } },
    });
    assert.equal(res.status, 400);
    const chat = await p.post({
      model: "opencode-route/t-chat-desig",
      messages: MSGS,
    });
    assert.equal(chat.status, 200);
    assert.equal((await chat.json()).choices[0].message.content, "LEAF-B");
    const rows = await waitForAttempts(p, 1);
    assert.equal(rows[0].provider_id, "leafB");
  } finally {
    await p.cleanup();
  }
});

test("embeddings serve a single string via the OpenAI default (#13675 P2)", async () => {
  const p = await startProxy();
  try {
    const res = await postPurpose(p, "/v1/embeddings", {
      model: "opencode-route/t-embed",
      input: "hello",
    });
    assert.equal(res.status, 200);
    const json = await res.json();
    assert.equal(json.object, "list");
    assert.equal(json.data.length, 1);
    assert.equal(json.data[0].index, 0);
    assert.deepEqual(json.data[0].embedding, [5, 0.5]);
    assert.equal(json.model, "opencode-route/t-embed");
  } finally {
    await p.cleanup();
  }
});

test("embeddings normalize the google single-item shape (#13675 P2)", async () => {
  const p = await startProxy();
  try {
    const res = await postPurpose(p, "/v1/embeddings", {
      model: "opencode-route/t-embed-goog",
      input: "hello",
    });
    assert.equal(res.status, 200);
    const json = await res.json();
    assert.equal(json.object, "list");
    assert.equal(json.data.length, 1);
    assert.deepEqual(json.data[0].embedding, [5, 0.5]);
    assert.ok(p.leafA.last.url.includes(":embedContent"));
  } finally {
    await p.cleanup();
  }
});

test("A2 splits oversized batches and re-joins in caller order (#13675 P2)", async () => {
  const p = await startProxy();
  try {
    const input = ["a", "bb", "ccc", "dddd", "eeeee"];
    const res = await postPurpose(p, "/v1/embeddings", {
      model: "opencode-route/t-embed-split",
      input,
    });
    assert.equal(res.status, 200);
    const json = await res.json();
    assert.equal(json.data.length, 5);
    for (let i = 0; i < 5; i++) {
      assert.equal(json.data[i].index, i);
      assert.deepEqual(json.data[i].embedding, [input[i].length, 0.5]);
    }
    assert.ok(!("errors" in json));
  } finally {
    await p.cleanup();
  }
});

test("A2 retryable sub-batch failure fails over to the next candidate (#13675 P2)", async () => {
  const p = await startProxy();
  try {
    p.leafA.mode = "rate";
    const ok = await postPurpose(p, "/v1/embeddings", {
      model: "opencode-route/t-embed",
      input: ["aa", "bbb"],
    });
    assert.equal(ok.status, 200);
    const okJson = await ok.json();
    assert.equal(okJson.data.length, 2);
    assert.deepEqual(okJson.data[0].embedding, [2, 0.5]);
    assert.deepEqual(okJson.data[1].embedding, [3, 0.5]);
    assert.equal(p.leafA.hits, 1);
    assert.equal(p.leafB.hits, 1);
  } finally {
    await p.cleanup();
  }
});

test("A2 non-retryable sub-batch failure answers 207 with index-faithful errors (#13675 P2)", async () => {
  const q = await startProxy();
  try {
    q.leafA.mode = "embed-bad-item";
    const res = await postPurpose(q, "/v1/embeddings", {
      model: "opencode-route/t-embed-split",
      input: ["aa", "bb", "bad-1", "bad-2", "cc"],
    });
    assert.equal(res.status, 207);
    const json = await res.json();
    assert.deepEqual(
      json.data.map((d) => d.index),
      [0, 1, 4],
    );
    assert.deepEqual(
      json.errors.map((e) => e.index),
      [2, 3],
    );
    assert.deepEqual(json.data[0].embedding, [2, 0.5]);
    assert.deepEqual(json.data[2].embedding, [2, 0.5]);
  } finally {
    await q.cleanup();
  }
});

test("rerank relays documents with scores (#13675 P2)", async () => {
  const p = await startProxy();
  try {
    const res = await postPurpose(p, "/v1/rerank", {
      model: "opencode-route/t-rerank",
      query: "what",
      documents: ["doc-a", "doc-b"],
    });
    assert.equal(res.status, 200);
    const json = await res.json();
    assert.equal(json.results.length, 2);
    assert.equal(json.results[0].index, 0);
    assert.ok(json.results[0].score > json.results[1].score);
    const bad = await postPurpose(p, "/v1/rerank", {
      model: "opencode-route/t-rerank",
      query: "",
      documents: [],
    });
    assert.equal(bad.status, 400);
  } finally {
    await p.cleanup();
  }
});

test("transcription relays the leaf answer bytes untouched (#13675 P2)", async () => {
  const p = await startProxy();
  try {
    const res = await postTranscription(p, "opencode-route/t-transcribe");
    assert.equal(res.status, 200);
    assert.equal((await res.json()).text, "transcribed-by-LEAF-A");
    const missing = await postPurpose(p, "/v1/audio/transcriptions", null, "--x\r\n");
    assert.equal(missing.status, 400);
  } finally {
    await p.cleanup();
  }
});

test("speech relays leaf audio bytes with the leaf content-type (#13675 P2)", async () => {
  const p = await startProxy();
  try {
    const res = await postPurpose(p, "/v1/audio/speech", {
      model: "opencode-route/t-speak",
      input: "say this",
      voice: "alloy",
    });
    assert.equal(res.status, 200);
    assert.match(res.headers.get("content-type"), /audio\/mpeg/);
    assert.equal(Buffer.from(await res.arrayBuffer()).toString("utf8"), "audio-by-LEAF-A");
  } finally {
    await p.cleanup();
  }
});

test("guard serves chat-shaped calls through the guard pool only (#13675 P2)", async () => {
  const p = await startProxy();
  try {
    const res = await postPurpose(p, "/v1/guard", {
      model: "opencode-route/t-guard",
      messages: MSGS,
    });
    assert.equal(res.status, 200);
    assert.equal((await res.json()).choices[0].message.content, "LEAF-A");
    assert.ok(p.leafA.last.url.endsWith("/chat/completions"));
    const mismatch = await postPurpose(p, "/v1/guard", {
      model: "opencode-route/t-ordered",
      messages: MSGS,
    });
    assert.equal(mismatch.status, 400);
    assert.match(await mismatch.text(), /cannot serve guard/);
  } finally {
    await p.cleanup();
  }
});

// ------------------------------------------------ metering wall (#13839)
//
// Fixture arithmetic: every successful fake-leaf call reports
// usage { prompt_tokens: 5, completion_tokens: 2 } = 7 counted
// tokens. leafFree's big group (model m-free) declares
// dailyTokens 28 with marginTokens 7, so its counted cap is 21 =
// exactly three calls; its small group (m-free2) declares 700.
// leafPaid is the unmetered paid tail behind the same model.

test("metering wall: counted usage walls the free leg at bucket minus margin and the route falls through (oracle: the ledger's own recorded usage, 7 tokens per fixture response)", async () => {
  const p = await startProxy();
  try {
    for (let i = 0; i < 3; i++) {
      const res = await p.post({ model: "opencode-route/t-metered", messages: MSGS });
      assert.equal(res.status, 200);
      assert.equal((await res.json()).choices[0].message.content, "LEAF-A");
    }
    await waitForAttempts(p, 3);
    // Counting accuracy: the ledger rows the proxy itself recorded
    // carry the fixture usage verbatim, and their input+output sum
    // is the counted total the wall judges.
    const db = new DatabaseSync(p.dbPath, { readOnly: true });
    const counted = db
      .prepare(
        "SELECT tokens_input, tokens_output FROM route_attempt WHERE provider_id = 'leafFree' ORDER BY id",
      )
      .all();
    db.close();
    assert.deepEqual(
      counted.map((r) => [r.tokens_input, r.tokens_output]),
      [
        [5, 2],
        [5, 2],
        [5, 2],
      ],
    );
    assert.equal(
      counted.reduce((s, r) => s + r.tokens_input + r.tokens_output, 0),
      21,
    );
    // The 4th turn must not serve via leafFree: 21 counted tokens
    // is the big group's cap (28 - 7 margin). No spill.
    const fourth = await p.post({ model: "opencode-route/t-metered", messages: MSGS });
    assert.equal(fourth.status, 200);
    assert.equal((await fourth.json()).choices[0].message.content, "LEAF-B");
    const after = p.attempts();
    assert.equal(
      after.filter((r) => r.provider_id === "leafFree").length,
      3,
      "a metered leg served after its bucket was spent",
    );
    assert.equal(after.filter((r) => r.provider_id === "leafPaid").length, 1);
    assert.match(p.log(), /metering wall excluded leafFree\/m-free/);
  } finally {
    await p.cleanup();
  }
});

test("metering wall: the cap is bucket minus margin, exactly, and reasoning tokens count once (oracle: seeded ledger counts at the boundary)", async () => {
  const PINNED = Date.UTC(2026, 0, 15, 12, 0, 0);
  const seed = (p, tokensInput, tokensOutput, tokensReasoning) => {
    const db = new DatabaseSync(p.dbPath);
    db.prepare(
      `INSERT INTO route_attempt
       (time_started, time_ended, route_id, provider_id, model_id, outcome, output_started, tokens_input, tokens_output, tokens_reasoning)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
    ).run(
      PINNED - 60_000,
      PINNED - 59_000,
      "t-metered-only",
      "leafFree",
      "m-free",
      "success",
      1,
      tokensInput,
      tokensOutput,
      tokensReasoning,
    );
    db.close();
  };
  // 20 counted tokens < cap 21: the leg still serves. The seeded
  // row carries 10 reasoning tokens inside its 10 output tokens
  // (the OpenAI usage shape usageFields records): counted once,
  // the total is 20 — double-counting the subset would read 30
  // and wall the leg here.
  const under = await startProxy({ nowMs: PINNED });
  try {
    seed(under, 10, 10, 10);
    const res = await under.post({ model: "opencode-route/t-metered-only", messages: MSGS });
    assert.equal(res.status, 200);
    assert.equal((await res.json()).choices[0].message.content, "LEAF-A");
  } finally {
    await under.cleanup();
  }
  // 21 counted tokens = cap 21 (28 dailyTokens - 7 margin): the
  // leg walls, and with no fall-through leg the request fails
  // closed instead of billing.
  const at = await startProxy({ nowMs: PINNED });
  try {
    seed(at, 11, 10, 3);
    const res = await at.post({ model: "opencode-route/t-metered-only", messages: MSGS });
    assert.equal(res.status, 502);
    assert.match((await res.json()).error.message, /no available members/);
    assert.equal(at.leafA.hits, 0, "the metered leg was called despite the spent bucket");
    assert.equal(
      at.attempts().filter((r) => r.provider_id === "leafFree").length,
      1,
      "only the seeded row exists; no new free-leg attempt was recorded",
    );
  } finally {
    await at.cleanup();
  }
});

test("metering wall: groups count separately (oracle: a spent big group walls only its own models)", async () => {
  const p = await startProxy();
  try {
    const db = new DatabaseSync(p.dbPath);
    const now = Date.now();
    db.prepare(
      `INSERT INTO route_attempt
       (time_started, time_ended, route_id, provider_id, model_id, outcome, output_started, tokens_input, tokens_output)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)`,
    ).run(now - 1000, now - 999, "t-metered-groups", "leafFree", "m-free", "success", 1, 500, 500);
    db.close();
    // m-free (big group, cap 21) is walled by the seeded 1000
    // tokens; m-free2 (small group, cap 700, count 0) still serves
    // on the same provider.
    const res = await p.post({ model: "opencode-route/t-metered-groups", messages: MSGS });
    assert.equal(res.status, 200);
    assert.equal((await res.json()).choices[0].message.content, "LEAF-A");
    const freeSuccesses = p
      .attempts()
      .filter((r) => r.provider_id === "leafFree" && r.outcome === "success");
    assert.equal(freeSuccesses.at(-1).model_id, "m-free2");
    // And the plain metered route falls through to the paid tail.
    const res2 = await p.post({ model: "opencode-route/t-metered", messages: MSGS });
    assert.equal(res2.status, 200);
    assert.equal((await res2.json()).choices[0].message.content, "LEAF-B");
  } finally {
    await p.cleanup();
  }
});

test("metering wall: the wall lifts at the 00:00 UTC reset with no operator action (oracle: the pinned clock's UTC day window)", async () => {
  const AFTER_MIDNIGHT = Date.UTC(2026, 0, 16, 0, 0, 30);
  const p = await startProxy({ nowMs: AFTER_MIDNIGHT });
  try {
    const db = new DatabaseSync(p.dbPath);
    // Yesterday's spend: 1000 counted tokens, started before
    // midnight — far past the cap, but in yesterday's window.
    db.prepare(
      `INSERT INTO route_attempt
       (time_started, time_ended, route_id, provider_id, model_id, outcome, output_started, tokens_input, tokens_output)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)`,
    ).run(
      Date.UTC(2026, 0, 15, 23, 59, 0),
      Date.UTC(2026, 0, 15, 23, 59, 1),
      "t-metered-only",
      "leafFree",
      "m-free",
      "success",
      1,
      600,
      400,
    );
    db.close();
    const res = await p.post({ model: "opencode-route/t-metered-only", messages: MSGS });
    assert.equal(res.status, 200);
    assert.equal((await res.json()).choices[0].message.content, "LEAF-A");
  } finally {
    await p.cleanup();
  }
});

// A minimal second spawn helper for the restart test: startProxy
// always builds a fresh fixture home, so a restart over the SAME
// config + ledger needs its own boot.
const spawnProxyOn = async ({ configPath, dbPath, home }) => {
  const port = await freePort();
  const env = {
    ...process.env,
    HOME: home,
    OPENCODE_ROUTE_PROXY_PORT: String(port),
    OPENCODE_ROUTE_PROXY_CONFIG: configPath,
    OPENCODE_ROUTE_PROXY_DB: dbPath,
    OPENCODE_ROUTE_PROXY_TIMEOUT_MS: "15000",
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
  return {
    post: (payload) =>
      fetch(`${base}/v1/chat/completions`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(payload),
      }),
    kill: () => child.kill("SIGKILL"),
  };
};

test("metering wall: the counted bucket survives a proxy restart (oracle: a fresh process over the same ledger still walls with zero new usage)", async () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "route-proxy-metering-restart-"));
  const leaf = await makeLeaf("LEAF-R", "key-r");
  try {
    const dbPath = path.join(tmp, "opencode.db");
    const db = new DatabaseSync(dbPath);
    db.exec(
      `CREATE TABLE credential (id integer PRIMARY KEY, integration_id text, label text, value text, active integer);
       ${ROUTE_ATTEMPT_SCHEMA}`,
    );
    db.close();
    const configPath = path.join(tmp, "opencode.jsonc");
    fs.writeFileSync(
      configPath,
      `{
  "provider": {
    "soloFree": {
      "npm": "@ai-sdk/openai-compatible",
      "options": { "baseURL": "http://127.0.0.1:${leaf.port}/v1", "apiKey": "key-r" },
      "tokenMetering": { "groups": { "g": { "dailyTokens": 21, "marginTokens": 0, "models": ["m-solo"] } } },
      "models": { "m-solo": {} }
    }
  },
  "experimental": {
    "model_routes": {
      "t-solo": { "selection": "ordered", "targets": ["soloFree/m-solo"] }
    }
  }
}`,
    );
    const first = await spawnProxyOn({ configPath, dbPath, home: tmp });
    try {
      for (let i = 0; i < 3; i++) {
        const res = await first.post({ model: "opencode-route/t-solo", messages: MSGS });
        assert.equal(res.status, 200);
        await res.text();
      }
      const walled = await first.post({ model: "opencode-route/t-solo", messages: MSGS });
      assert.equal(walled.status, 502);
      await walled.text();
      assert.equal(leaf.hits, 3);
    } finally {
      first.kill();
    }
    // A fresh process over the same ledger: the spent bucket is
    // still spent — the counter is the ledger, not memory.
    const second = await spawnProxyOn({ configPath, dbPath, home: tmp });
    try {
      const res = await second.post({ model: "opencode-route/t-solo", messages: MSGS });
      assert.equal(res.status, 502);
      await res.text();
      assert.equal(leaf.hits, 3, "the restarted proxy served a metered leg off a spent bucket");
    } finally {
      second.kill();
    }
  } finally {
    leaf.server.close();
  }
});
