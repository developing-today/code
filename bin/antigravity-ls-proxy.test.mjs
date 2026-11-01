// Tests for antigravity-ls-proxy's model resolution (#686).
// Run: node --test bin/antigravity-ls-proxy.test.mjs
//
// The proxy talks to an Antigravity language server over HTTP RPC. These
// tests run a stub language server, point the proxy at it via
// ANTIGRAVITY_LS_ADDRESS / ANTIGRAVITY_CSRF_TOKEN (the env attach path in
// detectRunningInstances), and capture the `model` value the proxy sends
// in GetModelResponse. A MODEL_MAP family entry carries metadata keys
// (`default`, `name`) besides effort -> enum, and an effort string that
// collides with one of those keys used to "resolve" to the metadata
// value — an effort name or a display name — instead of a model enum.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import http from 'node:http';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import test, { after, before } from 'node:test';
import { fileURLToPath } from 'node:url';

const binDir = path.dirname(fileURLToPath(import.meta.url));

const freePort = () => new Promise((resolve, reject) => {
  const srv = net.createServer();
  srv.once('error', reject);
  srv.listen(0, '127.0.0.1', () => {
    const { port } = srv.address();
    srv.close(() => resolve(port));
  });
});

let lsServer;
let proxyChild;
let proxyPort;
let capturedModel;
let capturedPrompt;
let stubResponse = 'stub answer';

const startStubLs = async () => {
  lsServer = http.createServer((req, res) => {
    let body = '';
    req.on('data', (c) => { body += c; });
    req.on('end', () => {
      res.writeHead(200, { 'Content-Type': 'application/json' });
      if (req.url.endsWith('/GetAvailableModels')) {
        res.end(JSON.stringify({ response: { models: {} } }));
      } else if (req.url.endsWith('/GetModelResponse')) {
        try {
          const parsed = JSON.parse(body);
          capturedModel = parsed.model;
          capturedPrompt = parsed.prompt;
        } catch { capturedModel = null; capturedPrompt = null; }
        res.end(JSON.stringify({ response: stubResponse }));
      } else {
        // Heartbeat and anything else: a bare 200 is all the proxy checks.
        res.end('{}');
      }
    });
  });
  const port = await freePort();
  await new Promise((resolve) => lsServer.listen(port, '127.0.0.1', resolve));
  return port;
};

before(async () => {
  const lsPort = await startStubLs();
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'ls-proxy-test-'));
  proxyPort = await freePort();
  proxyChild = spawn(process.execPath, [path.join(binDir, 'antigravity-ls-proxy.mjs')], {
    env: {
      ...process.env,
      ANTIGRAVITY_LS_PROXY_PORT: String(proxyPort),
      ANTIGRAVITY_LS_ADDRESS: `127.0.0.1:${lsPort}`,
      ANTIGRAVITY_CSRF_TOKEN: 'test-token',
      XDG_STATE_HOME: path.join(tmp, 'state')
    },
    stdio: ['ignore', 'pipe', 'pipe']
  });
  let log = '';
  proxyChild.stdout.on('data', (d) => { log += d; });
  proxyChild.stderr.on('data', (d) => { log += d; });
  const deadline = Date.now() + 10000;
  for (;;) {
    try {
      const res = await fetch(`http://127.0.0.1:${proxyPort}/health`);
      if (res.ok) break;
    } catch {}
    if (Date.now() > deadline) throw new Error(`proxy did not start: ${log}`);
    await new Promise((r) => setTimeout(r, 100));
  }
});

after(() => {
  if (proxyChild) proxyChild.kill('SIGKILL');
  if (lsServer) lsServer.close();
});

const chat = async (payload) => {
  capturedModel = undefined;
  capturedPrompt = undefined;
  const res = await fetch(`http://127.0.0.1:${proxyPort}/v1/chat/completions`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      messages: [{ role: 'user', content: 'hello' }],
      ...payload
    })
  });
  return res;
};

const echoTools = [{
  type: 'function',
  function: {
    name: 'run_echo',
    description: 'echo a word',
    parameters: {
      type: 'object',
      properties: { word: { type: 'string' }, count: { type: 'number' } },
      required: ['word']
    }
  }
}];

test("effort 'default' falls back to the family default enum, not the string 'medium'", async () => {
  const res = await chat({ model: 'gemini-3.8-flash', reasoning_effort: 'default' });
  assert.equal(res.status, 200);
  assert.equal(capturedModel, 'MODEL_PLACEHOLDER_M319');
});

test("effort 'name' falls back to the family default enum, not the display name", async () => {
  const res = await chat({ model: 'gemini-3.8-flash', reasoning_effort: 'name' });
  assert.equal(res.status, 200);
  assert.equal(capturedModel, 'MODEL_PLACEHOLDER_M319');
});

test('a valid effort still resolves to its own enum', async () => {
  const res = await chat({ model: 'gemini-3.8-flash', reasoning_effort: 'high' });
  assert.equal(res.status, 200);
  assert.equal(capturedModel, 'MODEL_PLACEHOLDER_M318');
});

test('an unoffered effort still falls back to the family default', async () => {
  const res = await chat({ model: 'gemini-3.1-pro', reasoning_effort: 'medium' });
  assert.equal(res.status, 200);
  assert.equal(capturedModel, 'MODEL_PLACEHOLDER_M37');
});

test('an unknown model is still refused with 400', async () => {
  const res = await chat({ model: 'no-such-model-xyz' });
  assert.equal(res.status, 400);
  assert.equal(capturedModel, undefined);
});

// --- Tool-call translation (#865) -------------------------------------------
// The language server returns plain text; the proxy must lift the tool
// calls models spell in that text into structured OpenAI tool_calls, or
// tool-driven clients (OpenCode) execute nothing.

test('the formatted prompt carries the tool-calling contract', async () => {
  stubResponse = 'ok';
  const res = await chat({ model: 'gemini-3.8-flash', tools: echoTools });
  assert.equal(res.status, 200);
  assert.match(capturedPrompt, /\[Tool Calling\]/);
  assert.match(capturedPrompt, /```tool_call/);
});

test('a canonical tool_call fence becomes structured tool_calls', async () => {
  stubResponse = '```tool_call\n{"name": "run_echo", "arguments": {"word": "banana", "count": 2}}\n```';
  const res = await chat({ model: 'gemini-3.8-flash', tools: echoTools });
  assert.equal(res.status, 200);
  const body = await res.json();
  const choice = body.choices[0];
  assert.equal(choice.finish_reason, 'tool_calls');
  assert.equal(choice.message.content, null);
  assert.equal(choice.message.tool_calls.length, 1);
  const call = choice.message.tool_calls[0];
  assert.equal(call.type, 'function');
  assert.equal(call.function.name, 'run_echo');
  assert.deepEqual(JSON.parse(call.function.arguments), { word: 'banana', count: 2 });
  assert.ok(call.id.startsWith('call_'));
});

test('a Claude-style invoke block becomes tool_calls and prose survives', async () => {
  stubResponse = 'Let me run that.\n\n<function_calls>\n<invoke name="run_echo">\n<parameter name="word">banana</parameter>\n<parameter name="count">3</parameter>\n</invoke>\n</function_calls>';
  const res = await chat({ model: 'gemini-3.8-flash', tools: echoTools });
  assert.equal(res.status, 200);
  const body = await res.json();
  const choice = body.choices[0];
  assert.equal(choice.finish_reason, 'tool_calls');
  assert.equal(choice.message.content, 'Let me run that.');
  assert.equal(choice.message.tool_calls.length, 1);
  assert.deepEqual(JSON.parse(choice.message.tool_calls[0].function.arguments), { word: 'banana', count: 3 });
});

test('a Gemini-style json fence becomes tool_calls', async () => {
  stubResponse = '```json\n{ "name": "run_echo", "arguments": { "word": "banana" } }\n```';
  const res = await chat({ model: 'gemini-3.8-flash', tools: echoTools });
  assert.equal(res.status, 200);
  const body = await res.json();
  assert.equal(body.choices[0].finish_reason, 'tool_calls');
  assert.equal(body.choices[0].message.tool_calls[0].function.name, 'run_echo');
});

test('a block naming an unoffered tool is left as text', async () => {
  stubResponse = '```tool_call\n{"name": "rm_rf", "arguments": {}}\n```';
  const res = await chat({ model: 'gemini-3.8-flash', tools: echoTools });
  assert.equal(res.status, 200);
  const body = await res.json();
  assert.equal(body.choices[0].finish_reason, 'stop');
  assert.equal(body.choices[0].message.tool_calls, undefined);
  assert.match(body.choices[0].message.content, /rm_rf/);
});

test('no tools in the request means no extraction', async () => {
  stubResponse = '```tool_call\n{"name": "run_echo", "arguments": {"word": "banana"}}\n```';
  const res = await chat({ model: 'gemini-3.8-flash' });
  assert.equal(res.status, 200);
  const body = await res.json();
  assert.equal(body.choices[0].finish_reason, 'stop');
  assert.equal(body.choices[0].message.tool_calls, undefined);
});

test('streaming emits a tool_calls delta and a tool_calls finish', async () => {
  stubResponse = '```tool_call\n{"name": "run_echo", "arguments": {"word": "banana"}}\n```';
  const res = await chat({ model: 'gemini-3.8-flash', tools: echoTools, stream: true });
  assert.equal(res.status, 200);
  const text = await res.text();
  const chunks = text.split('\n')
    .filter(l => l.startsWith('data: ') && !l.includes('[DONE]'))
    .map(l => JSON.parse(l.slice('data: '.length)));
  const toolChunks = chunks.filter(c => c.choices[0]?.delta?.tool_calls);
  assert.equal(toolChunks.length, 1);
  assert.equal(toolChunks[0].choices[0].delta.tool_calls[0].function.name, 'run_echo');
  const finish = chunks[chunks.length - 1];
  assert.equal(finish.choices[0].finish_reason, 'tool_calls');
});

test('non-object JSON body is a 400 and the proxy stays alive (#2816)', async () => {
  for (const bad of ['null', '[1,2]', '"x"', '42']) {
    const res = await fetch(`http://127.0.0.1:${proxyPort}/v1/chat/completions`, {
      method: 'POST', headers: { 'Content-Type': 'application/json' }, body: bad
    });
    assert.equal(res.status, 400, `body ${bad} must be a 400, got ${res.status}`);
    await res.text();
  }
  const alive = await fetch(`http://127.0.0.1:${proxyPort}/v1/chat/completions`, {
    method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{bad json'
  });
  assert.equal(alive.status, 400, 'proxy must still serve after non-object bodies');
  await alive.text();
});

// --- RPC timeouts (#2102) ---------------------------------------------------
// A language server that passes heartbeat but never answers an RPC used to
// hang /v1/models and every chat request forever: callRpc fetched with no
// timeout. These tests run their own stub + proxy pairs with short injected
// budgets (ANTIGRAVITY_LS_RPC_TIMEOUTS / ANTIGRAVITY_LS_CASCADE_DEADLINE_MS)
// so nothing waits for the production budgets.

const startTimeoutRig = async ({ hang = [], delayMs = {}, cascadeSteps = [], timeouts = {}, cascadeDeadlineMs = 30_000 } = {}) => {
  const hangSet = new Set(hang);
  const stub = http.createServer((req, res) => {
    let body = '';
    req.on('data', (c) => { body += c; });
    req.on('end', () => {
      const method = req.url.split('/').pop();
      if (hangSet.has(method)) return; // heartbeat-ok, RPC never answers
      const respond = () => {
        if (res.destroyed) return;
        res.writeHead(200, { 'Content-Type': 'application/json' });
        if (method === 'GetAvailableModels') res.end(JSON.stringify({ response: { models: {} } }));
        else if (method === 'GetModelResponse') res.end(JSON.stringify({ response: 'slow but alive' }));
        else if (method === 'StartCascade') res.end(JSON.stringify({ cascadeId: 'cascade-timeout-test' }));
        else if (method === 'GetCascadeTrajectorySteps') res.end(JSON.stringify({ steps: cascadeSteps }));
        else res.end('{}');
      };
      const d = delayMs[method] || 0;
      if (d) setTimeout(respond, d); else respond();
    });
  });
  const lsPort = await freePort();
  await new Promise((resolve) => stub.listen(lsPort, '127.0.0.1', resolve));
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'ls-proxy-timeout-test-'));
  const port = await freePort();
  const child = spawn(process.execPath, [path.join(binDir, 'antigravity-ls-proxy.mjs')], {
    env: {
      ...process.env,
      ANTIGRAVITY_LS_PROXY_PORT: String(port),
      ANTIGRAVITY_LS_ADDRESS: `127.0.0.1:${stub.address().port}`,
      ANTIGRAVITY_CSRF_TOKEN: 'test-token',
      ANTIGRAVITY_LS_RPC_TIMEOUTS: JSON.stringify(timeouts),
      ANTIGRAVITY_LS_CASCADE_DEADLINE_MS: String(cascadeDeadlineMs),
      XDG_STATE_HOME: path.join(tmp, 'state')
    },
    stdio: ['ignore', 'pipe', 'pipe']
  });
  let log = '';
  child.stdout.on('data', (d) => { log += d; });
  child.stderr.on('data', (d) => { log += d; });
  const deadline = Date.now() + 10000;
  for (;;) {
    try {
      const res = await fetch(`http://127.0.0.1:${port}/health`);
      if (res.ok) break;
    } catch {}
    if (Date.now() > deadline) throw new Error(`timeout-rig proxy did not start: ${log}`);
    await new Promise((r) => setTimeout(r, 100));
  }
  return {
    port,
    close: () => { child.kill('SIGKILL'); stub.close(); }
  };
};

test('a hung GetAvailableModels still serves the static catalog, bounded (#2102)', async () => {
  const rig = await startTimeoutRig({ hang: ['GetAvailableModels'], timeouts: { GetAvailableModels: 200 } });
  try {
    const t0 = Date.now();
    const res = await fetch(`http://127.0.0.1:${rig.port}/v1/models`);
    const elapsed = Date.now() - t0;
    assert.equal(res.status, 200);
    const body = await res.json();
    assert.ok(body.data.some((m) => m.id === 'gemini-3.8-flash'), 'static MODEL_MAP catalog must be served');
    assert.ok(elapsed < 5000, `models took ${elapsed}ms; the catalog RPC must be bounded`);
  } finally { rig.close(); }
});

test('a hung GetModelResponse returns a clean bounded API error (#2102)', async () => {
  const rig = await startTimeoutRig({ hang: ['GetModelResponse'], timeouts: { GetModelResponse: 200 } });
  try {
    const t0 = Date.now();
    const res = await fetch(`http://127.0.0.1:${rig.port}/v1/chat/completions`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ model: 'gemini-3.8-flash', messages: [{ role: 'user', content: 'hello' }] })
    });
    const elapsed = Date.now() - t0;
    assert.notEqual(res.status, 200);
    const body = await res.json();
    assert.match(JSON.stringify(body), /GetModelResponse timed out after 200ms/);
    assert.ok(elapsed < 5000, `chat took ${elapsed}ms; the RPC must be bounded`);
  } finally { rig.close(); }
});

test('a slow-but-alive RPC below its budget still succeeds (#2102)', async () => {
  const rig = await startTimeoutRig({ delayMs: { GetModelResponse: 100 }, timeouts: { GetModelResponse: 2000 } });
  try {
    const res = await fetch(`http://127.0.0.1:${rig.port}/v1/chat/completions`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ model: 'gemini-3.8-flash', messages: [{ role: 'user', content: 'hello' }] })
    });
    assert.equal(res.status, 200);
    const body = await res.json();
    assert.equal(body.choices[0].message.content, 'slow but alive');
  } finally { rig.close(); }
});

test('cascade polling stops at the elapsed deadline, not an iteration count (#2102)', async () => {
  const rig = await startTimeoutRig({
    timeouts: { GetCascadeTrajectorySteps: 1000 },
    cascadeDeadlineMs: 600,
    cascadeSteps: []
  });
  try {
    const t0 = Date.now();
    const res = await fetch(`http://127.0.0.1:${rig.port}/v1/chat/completions`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ model: 'gemini-3.8-flash', mode: 'cascade', messages: [{ role: 'user', content: 'hello' }] })
    });
    const elapsed = Date.now() - t0;
    assert.notEqual(res.status, 200);
    const body = await res.json();
    assert.match(JSON.stringify(body), /Cascade execution timed out after 600ms/);
    assert.ok(elapsed < 5000, `cascade took ${elapsed}ms; the deadline must bound it`);
  } finally { rig.close(); }
});

test('a healthy cascade still completes (#2102)', async () => {
  const rig = await startTimeoutRig({
    cascadeSteps: [{
      type: 'CORTEX_STEP_TYPE_PLANNER_RESPONSE',
      status: 'CORTEX_STEP_STATUS_DONE',
      plannerResponse: { response: 'cascade answer' },
      metadata: {}
    }]
  });
  try {
    const res = await fetch(`http://127.0.0.1:${rig.port}/v1/chat/completions`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ model: 'gemini-3.8-flash', mode: 'cascade', messages: [{ role: 'user', content: 'hello' }] })
    });
    assert.equal(res.status, 200);
    const body = await res.json();
    assert.equal(body.choices[0].message.content, 'cascade answer');
  } finally { rig.close(); }
});
