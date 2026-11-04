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

// --- opencode#1548: bare array schemas ------------------------------------
// A tool property declared {type: 'array'} with no items is legal JSON
// Schema (unconstrained elements) but the upstream proto validator
// rejects an ARRAY schema without items ("properties[paths].items:
// missing field"), which 400'd every tool-carrying session through this
// door. Conversion must synthesize an unconstrained items for any ARRAY
// node that carries neither items nor prefixItems.

const lastFunctionDeclarations = () => {
  const calls = generateCalls();
  assert.ok(calls.length > 0, 'expected at least one upstream generate call');
  return calls[calls.length - 1].body.request.tools[0].functionDeclarations;
};

const assertArraysCarryItems = (node, where) => {
  if (Array.isArray(node)) { node.forEach((v, i) => assertArraysCarryItems(v, `${where}[${i}]`)); return; }
  if (!node || typeof node !== 'object') return;
  if (node.type === 'ARRAY') {
    assert.ok(node.items !== undefined || node.prefixItems !== undefined, `${where}: ARRAY schema without items`);
  }
  for (const [k, v] of Object.entries(node)) assertArraysCarryItems(v, `${where}.${k}`);
};

test('bare array tool properties reach upstream with synthesized items (opencode#1548)', async () => {
  const tools = [{
    type: 'function',
    function: {
      name: 'collect_files',
      description: 'collect files',
      parameters: {
        type: 'object',
        properties: {
          paths: { type: 'array', description: 'paths to collect' },
          matrix: { type: 'array', items: { type: 'array' } },
          choice: { anyOf: [{ type: 'array' }, { type: 'string' }] },
          tuple: { type: 'array', items: [{ type: 'string' }, { type: 'number' }] },
          named: { type: 'array', items: { type: 'string' } }
        },
        required: ['paths']
      }
    }
  }];
  const { status } = await chat({ tools });
  assert.equal(status, 200);
  const decls = lastFunctionDeclarations();
  assert.equal(decls.length, 1);
  const props = decls[0].parameters.properties;
  assert.deepEqual(props.paths, { type: 'ARRAY', description: 'paths to collect', items: {} });
  assert.deepEqual(props.named, { type: 'ARRAY', items: { type: 'STRING' } });
  assertArraysCarryItems(decls[0].parameters, 'parameters');
});

// --- opencode#1550: anyOf folds to oneOf for Claude targets ---------------
// For Claude models the upstream re-serializes the proto Schema into an
// Anthropic input_schema, and Anthropic's validator rejects anyOf in any
// position ("tools.N.custom.input_schema: JSON schema is invalid. It must
// match JSON Schema draft 2020-12"), which 400'd workhorse sessions on
// the Claude leaf all day 2026-11-03 (any tool with a zod-union or
// nullable-anyOf property). Conversion for Claude targets must emit
// oneOf instead; Gemini targets keep anyOf verbatim.

const assertNoKey = (node, key, where) => {
  if (Array.isArray(node)) { node.forEach((v, i) => assertNoKey(v, key, `${where}[${i}]`)); return; }
  if (!node || typeof node !== 'object') return;
  assert.ok(!(key in node), `${where}: unexpected ${key}`);
  for (const [k, v] of Object.entries(node)) assertNoKey(v, key, `${where}.${k}`);
};

const UNION_TOOLS = [{
  type: 'function',
  function: {
    name: 'probe_union',
    description: 'union-shaped parameters',
    parameters: {
      type: 'object',
      properties: {
        nullable_note: { anyOf: [{ type: 'string' }, { type: 'null' }] },
        either: { anyOf: [{ type: 'string' }, { type: 'number' }] },
        listed: { type: 'array', items: { anyOf: [{ type: 'string' }, { type: 'boolean' }] } },
        pick: { oneOf: [{ type: 'string' }, { type: 'number' }] }
      }
    }
  }
}];

test('anyOf tool schemas fold to oneOf for claude models (opencode#1550)', async () => {
  const { status } = await chat({ model: 'claude-sonnet-5-5', tools: UNION_TOOLS });
  assert.equal(status, 200);
  const decls = lastFunctionDeclarations();
  assert.equal(decls.length, 1);
  const props = decls[0].parameters.properties;
  assert.deepEqual(props.nullable_note, { oneOf: [{ type: 'STRING' }, { type: 'NULL' }] });
  assert.deepEqual(props.either, { oneOf: [{ type: 'STRING' }, { type: 'NUMBER' }] });
  assert.deepEqual(props.listed, { type: 'ARRAY', items: { oneOf: [{ type: 'STRING' }, { type: 'BOOLEAN' }] } });
  assert.deepEqual(props.pick, { oneOf: [{ type: 'STRING' }, { type: 'NUMBER' }] });
  assertNoKey(decls[0].parameters, 'anyOf', 'parameters');
});

test('anyOf tool schemas stay anyOf for gemini models (opencode#1550)', async () => {
  const { status } = await chat({ tools: UNION_TOOLS });
  assert.equal(status, 200);
  const decls = lastFunctionDeclarations();
  const props = decls[0].parameters.properties;
  assert.deepEqual(props.nullable_note, { anyOf: [{ type: 'STRING' }, { type: 'NULL' }] });
  assert.deepEqual(props.either, { anyOf: [{ type: 'STRING' }, { type: 'NUMBER' }] });
  assert.deepEqual(props.listed, { type: 'ARRAY', items: { anyOf: [{ type: 'STRING' }, { type: 'BOOLEAN' }] } });
  assert.deepEqual(props.pick, { oneOf: [{ type: 'STRING' }, { type: 'NUMBER' }] });
});

// --- opencode#1550 (part 2): tool-call ids ride the history parts --------
// For Claude models the upstream's v1internal->Anthropic conversion emits
// a tool_use block per history functionCall part and a tool_result per
// functionResponse part, and Anthropic requires both ids
// ("messages.N.content.M.tool_use.id: Field required", then
// "tool_result.tool_use_id: Field required") -- which 400'd every Claude
// session continuation carrying a tool call. Conversion for Claude
// targets must carry the client's tool_call ids on both part kinds;
// Gemini targets keep the historic id-less parts.

const HISTORY = [
  { role: 'user', content: 'list the files' },
  { role: 'assistant', content: '', tool_calls: [{ id: 'call_1', type: 'function', function: { name: 'get_weather', arguments: '{"city":"Oslo"}' } }] },
  { role: 'tool', tool_call_id: 'call_1', name: 'get_weather', content: '{"temp": 3}' },
  { role: 'user', content: 'thanks' }
];

test('history tool calls carry their ids for claude models (opencode#1550)', async () => {
  const res = await fetch('http://127.0.0.1:' + proxyPort + '/v1/chat/completions', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ model: 'claude-sonnet-5-5', messages: HISTORY, tools: TOOLS })
  });
  assert.equal(res.status, 200);
  await res.json();
  const calls = generateCalls();
  const contents = calls[calls.length - 1].body.request.contents;
  const callPart = contents.flatMap(c => c.parts).find(p => p.functionCall);
  const respPart = contents.flatMap(c => c.parts).find(p => p.functionResponse);
  assert.equal(callPart.functionCall.id, 'call_1');
  assert.equal(respPart.functionResponse.id, 'call_1');
});

test('history tool calls stay id-less for gemini models (opencode#1550)', async () => {
  const res = await fetch('http://127.0.0.1:' + proxyPort + '/v1/chat/completions', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ model: 'gemini-3.8-flash', messages: HISTORY, tools: TOOLS })
  });
  assert.equal(res.status, 200);
  await res.json();
  const calls = generateCalls();
  const contents = calls[calls.length - 1].body.request.contents;
  const callPart = contents.flatMap(c => c.parts).find(p => p.functionCall);
  const respPart = contents.flatMap(c => c.parts).find(p => p.functionResponse);
  assert.ok(!('id' in callPart.functionCall), 'gemini functionCall must not carry id');
  assert.ok(!('id' in respPart.functionResponse), 'gemini functionResponse must not carry id');
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
