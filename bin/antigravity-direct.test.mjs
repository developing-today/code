// Tests for antigravity-direct's tool_choice translation (code#2822).
// A specific-function tool_choice must reach the upstream as
// {mode: 'ANY', allowedFunctionNames: [name]} — never flattened to bare
// ANY — and a choice naming an undeclared or malformed function must be
// rejected with a 400 before any upstream request. String forms and the
// no-choice case keep their historic shapes.
//
// Harness mirrors bin/muse-proxy.test.mjs: the real proxy is spawned
// against a stub upstream that records every payload, with a stub
// `secret-tool` (via SECRET_TOOL_PATH) handing it a far-future token so
// no keyring or network is touched.
// Run: node --test bin/antigravity-direct.test.mjs
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import http from 'node:http';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import test, { before, after } from 'node:test';
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

const TOOLS = [
  { type: 'function', function: { name: 'get_weather', parameters: { type: 'object', properties: {} } } },
  { type: 'function', function: { name: 'send_email', parameters: { type: 'object', properties: {} } } }
];

let upstream;
let upstreamPort;
let proxy;
let proxyPort;
let tmpDir;
const recorded = []; // { path, body } for every upstream POST

const generateCalls = () => recorded.filter(r => r.path.includes('streamGenerateContent'));
const lastToolConfig = () => {
  const calls = generateCalls();
  assert.ok(calls.length > 0, 'expected at least one upstream generate call');
  return calls[calls.length - 1].body.request.toolConfig;
};

before(async () => {
  tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'antigravity-direct-test-'));

  // Stub secret-tool: prints a keyring-shaped credential whose access
  // token is far from expiry, so the proxy never refreshes.
  const stubTool = path.join(tmpDir, 'secret-tool');
  const expiry = new Date(Date.now() + 3600e3).toISOString();
  fs.writeFileSync(stubTool, '#!/usr/bin/env node\nconsole.log(JSON.stringify({ token: { access_token: "stub-access-token", refresh_token: "stub-refresh-token", expiry: ' + JSON.stringify(expiry) + ' }, auth_method: "consumer" }));\n');
  fs.chmodSync(stubTool, 0o755);

  // Stub upstream: records payloads; answers fetchAvailableModels with
  // an empty catalog and streamGenerateContent with one minimal SSE
  // text event.
  upstreamPort = await freePort();
  upstream = http.createServer((req, res) => {
    const chunks = [];
    req.on('data', c => chunks.push(c));
    req.on('end', () => {
      let body = null;
      try { body = JSON.parse(Buffer.concat(chunks).toString('utf8') || '{}'); } catch {}
      recorded.push({ path: req.url, body });
      if (req.url.includes('fetchAvailableModels')) {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end('{}');
      } else if (req.url.includes('streamGenerateContent')) {
        res.writeHead(200, { 'Content-Type': 'text/event-stream' });
        res.end('data: ' + JSON.stringify({ response: { candidates: [{ content: { parts: [{ text: 'ok' }] }, finishReason: 'STOP' }] } }) + '\n\n');
      } else {
        res.writeHead(404);
        res.end();
      }
    });
  });
  await new Promise(r => upstream.listen(upstreamPort, '127.0.0.1', r));

  proxyPort = await freePort();
  proxy = spawn(process.execPath, [path.join(binDir, 'antigravity-direct.mjs')], {
    env: {
      ...process.env,
      ANTIGRAVITY_DIRECT_HOST: '127.0.0.1',
      ANTIGRAVITY_DIRECT_PORT: String(proxyPort),
      ANTIGRAVITY_DIRECT_UPSTREAM: 'http://127.0.0.1:' + upstreamPort,
      SECRET_TOOL_PATH: stubTool,
      XDG_STATE_HOME: tmpDir
    },
    stdio: ['ignore', 'ignore', 'pipe']
  });
  proxy.stderr.on('data', () => {});
  const deadline = Date.now() + 30000;
  for (;;) {
    try {
      const r = await fetch('http://127.0.0.1:' + proxyPort + '/healthz');
      if (r.ok) break;
    } catch {}
    if (Date.now() > deadline) throw new Error('proxy did not come up');
    await new Promise(r => setTimeout(r, 100));
  }
});

after(() => {
  if (proxy) proxy.kill('SIGKILL');
  if (upstream) upstream.close();
  if (tmpDir) fs.rmSync(tmpDir, { recursive: true, force: true });
});

const chat = async (extra) => {
  const res = await fetch('http://127.0.0.1:' + proxyPort + '/v1/chat/completions', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ model: 'gemini-3.8-flash', messages: [{ role: 'user', content: 'weather?' }], ...extra })
  });
  return { status: res.status, body: await res.json() };
};

test('specific-function tool_choice pins allowedFunctionNames', async () => {
  const { status } = await chat({ tools: TOOLS, tool_choice: { type: 'function', function: { name: 'get_weather' } } });
  assert.equal(status, 200);
  assert.deepEqual(lastToolConfig(), { functionCallingConfig: { mode: 'ANY', allowedFunctionNames: ['get_weather'] } });
});

test('string tool_choice forms keep their bare modes', async () => {
  for (const [choice, mode] of [['none', 'NONE'], ['auto', 'AUTO'], ['required', 'ANY']]) {
    const { status } = await chat({ tools: TOOLS, tool_choice: choice });
    assert.equal(status, 200, choice);
    assert.deepEqual(lastToolConfig(), { functionCallingConfig: { mode } }, choice);
  }
});

test('no tool_choice sends no toolConfig', async () => {
  const { status } = await chat({ tools: TOOLS });
  assert.equal(status, 200);
  assert.equal(lastToolConfig(), undefined);
});

test('tool_choice naming an undeclared function is a 400 and never reaches upstream', async () => {
  const beforeCount = generateCalls().length;
  const { status, body } = await chat({ tools: TOOLS, tool_choice: { type: 'function', function: { name: 'delete_everything' } } });
  assert.equal(status, 400);
  assert.equal(body.error.type, 'invalid_request_error');
  assert.match(body.error.message, /delete_everything/);
  assert.equal(generateCalls().length, beforeCount);
});

test('malformed specific-function tool_choice is a 400 and never reaches upstream', async () => {
  const beforeCount = generateCalls().length;
  for (const bad of [{ type: 'function', function: {} }, { type: 'function' }, {}]) {
    const { status, body } = await chat({ tools: TOOLS, tool_choice: bad });
    assert.equal(status, 400, JSON.stringify(bad));
    assert.equal(body.error.type, 'invalid_request_error');
  }
  assert.equal(generateCalls().length, beforeCount);
});

test('specific-function tool_choice with no declared tools is a 400', async () => {
  const beforeCount = generateCalls().length;
  const { status, body } = await chat({ tool_choice: { type: 'function', function: { name: 'get_weather' } } });
  assert.equal(status, 400);
  assert.equal(body.error.type, 'invalid_request_error');
  assert.equal(generateCalls().length, beforeCount);
});

// --- #2823: env parsing is strict -----------------------------------------
// A bad ANTIGRAVITY_DIRECT_PORT used to reach listen() as an anonymous
// ERR_SOCKET_BAD_PORT (or bind 0 / a hex-decoded port), and a bad
// ANTIGRAVITY_DIRECT_TIMEOUT_MS started healthy then 502'd every
// request. Both must now fail at startup, naming the variable and value.

const stubSecretTool = () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'antigravity-direct-env-'));
  const file = path.join(dir, 'secret-tool');
  const expiry = new Date(Date.now() + 3600e3).toISOString();
  fs.writeFileSync(file, '#!/usr/bin/env node\nconsole.log(JSON.stringify({ token: { access_token: "stub-access-token", refresh_token: "stub-refresh-token", expiry: ' + JSON.stringify(expiry) + ' }, auth_method: "consumer" }));\n');
  fs.chmodSync(file, 0o755);
  return { dir, file };
};

const spawnProxy = (env) => {
  const stub = stubSecretTool();
  const child = spawn(process.execPath, [path.join(binDir, 'antigravity-direct.mjs')], {
    env: {
      ...process.env,
      ANTIGRAVITY_DIRECT_HOST: '127.0.0.1',
      ANTIGRAVITY_DIRECT_UPSTREAM: 'http://127.0.0.1:1',
      SECRET_TOOL_PATH: stub.file,
      XDG_STATE_HOME: stub.dir,
      ...env
    },
    stdio: ['ignore', 'ignore', 'pipe']
  });
  let stderr = '';
  child.stderr.on('data', (c) => { stderr += c; });
  const exited = new Promise((resolve) => child.on('exit', (code, signal) => resolve({ code, signal, stderr })));
  const cleanup = () => fs.rmSync(stub.dir, { recursive: true, force: true });
  return { child, exited, cleanup };
};

const rejectedCase = (name, raw, pattern) => {
  test(`${name}=${raw} is rejected at startup naming the variable`, async () => {
    const { child, exited, cleanup } = spawnProxy({ [name]: raw });
    const timer = setTimeout(() => child.kill('SIGKILL'), 15000);
    const { code, stderr } = await exited;
    clearTimeout(timer);
    cleanup();
    assert.equal(code, 1, `expected exit 1, got ${code}; stderr=${stderr}`);
    assert.match(stderr, new RegExp(`${name}="${raw}"`.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')));
    assert.match(stderr, pattern);
    // A Node listen/timer error would name none of the above.
    assert.doesNotMatch(stderr, /ERR_SOCKET_BAD_PORT|RangeError|TimeoutOverflowWarning/);
  });
};

for (const raw of ['abc', '18941abc', '18941.9', '0', '99999', '0x4a35']) {
  rejectedCase('ANTIGRAVITY_DIRECT_PORT', raw, /not a valid port \(expected an integer 1-65535\)/);
}
for (const raw of ['abc', '-5000', '3000000000']) {
  rejectedCase('ANTIGRAVITY_DIRECT_TIMEOUT_MS', raw, /not a valid timeout \(expected an integer 1-2147483647 ms\)/);
}

test('valid port with a 300ms timeout serves and times a hanging upstream out', async () => {
  const hanging = http.createServer(() => {}); // accepts, never responds
  const hangingPort = await freePort();
  await new Promise(r => hanging.listen(hangingPort, '127.0.0.1', r));
  const port = await freePort();
  const { child, cleanup } = spawnProxy({
    ANTIGRAVITY_DIRECT_PORT: String(port),
    ANTIGRAVITY_DIRECT_TIMEOUT_MS: '300',
    ANTIGRAVITY_DIRECT_UPSTREAM: 'http://127.0.0.1:' + hangingPort
  });
  try {
    const deadline = Date.now() + 30000;
    for (;;) {
      try {
        const r = await fetch('http://127.0.0.1:' + port + '/healthz');
        if (r.ok) break;
      } catch {}
      if (Date.now() > deadline) throw new Error('proxy did not come up');
      await new Promise(r => setTimeout(r, 100));
    }
    const started = Date.now();
    const res = await fetch('http://127.0.0.1:' + port + '/v1/chat/completions', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ model: 'gemini-3.8-flash', messages: [{ role: 'user', content: 'hi' }] })
    });
    const body = await res.json();
    const elapsed = Date.now() - started;
    assert.equal(res.status, 502);
    assert.match(JSON.stringify(body), /upstream timeout after 300ms/);
    assert.ok(elapsed >= 250 && elapsed < 5000, `expected ~300ms timeout, took ${elapsed}ms`);
  } finally {
    child.kill('SIGKILL');
    hanging.close();
    cleanup();
  }
});
