// Tests for the OpenCode ACP-to-HTTP adapter (#1243).
// Run: node --test bin/opencode-acp-adapter.test.mjs
//
// A scripted ACP client (this file) drives the adapter over stdio
// against a stub OpenCode serve HTTP API, covering: initialize,
// session/new -> POST /session (cwd as directory), a streamed prompt
// round-trip (prompt_async + SSE -> session/update notifications),
// a permission round-trip (permission.updated ->
// session/request_permission -> POST permissions), and cancel
// (session/cancel -> POST abort, prompt resolves "cancelled").
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import http from 'node:http';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const binDir = path.dirname(fileURLToPath(import.meta.url));
const SESSION_ID = 'ses_test1243';

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

// ---------------------------------------------------------------------------
// Stub serve API. `script` runs when prompt_async arrives; it gets
// `emit(event)` to push SSE events to the adapter.
// ---------------------------------------------------------------------------
function startStubServe(script) {
  const calls = [];
  const sseClients = new Set();
  const emit = (event) => {
    const payload = `data: ${JSON.stringify(event)}\n\n`;
    for (const res of sseClients) res.write(payload);
  };
  const server = http.createServer((req, res) => {
    const url = new URL(req.url, 'http://127.0.0.1');
    let raw = '';
    req.on('data', (chunk) => {
      raw += chunk;
    });
    req.on('end', () => {
      const body = raw ? JSON.parse(raw) : null;
      calls.push({ method: req.method, pathname: url.pathname, query: url.searchParams, body });
      if (req.method === 'GET' && url.pathname === '/event') {
        res.writeHead(200, { 'Content-Type': 'text/event-stream', 'Cache-Control': 'no-cache' });
        res.flushHeaders();
        sseClients.add(res);
        // NB: req 'close' fires as soon as the (bodyless) request is
        // consumed, long before the stream ends — unregister on the
        // response closing instead.
        res.on('close', () => sseClients.delete(res));
        return;
      }
      if (req.method === 'POST' && url.pathname === '/session') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ id: SESSION_ID, directory: url.searchParams.get('directory') }));
        return;
      }
      if (req.method === 'POST' && url.pathname === `/session/${SESSION_ID}/prompt_async`) {
        res.writeHead(204);
        res.end();
        setImmediate(() => script(emit, body));
        return;
      }
      if (req.method === 'POST' && /\/permissions\//.test(url.pathname)) {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end('true');
        return;
      }
      if (req.method === 'POST' && url.pathname === `/session/${SESSION_ID}/abort`) {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end('true');
        emit({ type: 'session.idle', properties: { sessionID: SESSION_ID } });
        return;
      }
      res.writeHead(404, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ error: 'not found' }));
    });
  });
  return new Promise((resolve) => {
    server.listen(0, '127.0.0.1', () => {
      resolve({ server, calls, port: server.address().port });
    });
  });
}

// ---------------------------------------------------------------------------
// Scripted ACP client over the adapter's stdio.
// ---------------------------------------------------------------------------
function startAdapter(serverPort) {
  const child = spawn(process.execPath, [
    path.join(binDir, 'opencode-acp-adapter.mjs'),
    '--server',
    `http://127.0.0.1:${serverPort}`,
  ]);
  const pending = new Map();
  const notifications = [];
  const clientRequests = [];
  let buffer = '';
  let nextId = 1;
  child.stdout.setEncoding('utf8');
  child.stdout.on('data', (chunk) => {
    buffer += chunk;
    let idx = buffer.indexOf('\n');
    while (idx >= 0) {
      const line = buffer.slice(0, idx).trim();
      buffer = buffer.slice(idx + 1);
      idx = buffer.indexOf('\n');
      if (!line) continue;
      const msg = JSON.parse(line);
      if (msg.method && msg.id !== undefined) {
        clientRequests.push(msg);
      } else if (msg.method) {
        notifications.push(msg);
      } else if (msg.id !== undefined && pending.has(msg.id)) {
        const { resolve, reject } = pending.get(msg.id);
        pending.delete(msg.id);
        if (msg.error) reject(new Error(msg.error.message));
        else resolve(msg.result);
      }
    }
  });
  const request = (method, params) => {
    const id = nextId;
    nextId += 1;
    return new Promise((resolve, reject) => {
      pending.set(id, { resolve, reject });
      child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\n`);
    });
  };
  const notify = (method, params) => {
    child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', method, params })}\n`);
  };
  const respond = (id, result) => {
    child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id, result })}\n`);
  };
  const waitFor = async (predicate, what, timeoutMs = 8000) => {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      const found = predicate();
      if (found) return found;
      await sleep(25);
    }
    throw new Error(`timed out waiting for ${what}`);
  };
  return { child, request, notify, respond, notifications, clientRequests, waitFor };
}

async function setup(t, script) {
  const stub = await startStubServe(script);
  const acp = startAdapter(stub.port);
  t.after(() => {
    acp.child.kill('SIGKILL');
    stub.server.close();
  });
  const init = await acp.request('initialize', { protocolVersion: 1, clientCapabilities: {} });
  return { stub, acp, init };
}

const textEvents = () => (emit) => {
  emit({
    type: 'message.part.updated',
    properties: {
      part: { id: 'part1', sessionID: SESSION_ID, type: 'text', text: 'Hello ' },
      delta: 'Hello ',
    },
  });
  emit({
    type: 'message.part.updated',
    properties: {
      part: { id: 'part1', sessionID: SESSION_ID, type: 'text', text: 'Hello world' },
      delta: 'world',
    },
  });
  emit({ type: 'session.idle', properties: { sessionID: SESSION_ID } });
};

test('initialize negotiates ACP v1 and advertises prompt capabilities', async (t) => {
  const { init } = await setup(t, textEvents());
  assert.equal(init.protocolVersion, 1);
  assert.equal(init.agentCapabilities.loadSession, false);
  assert.equal(init.agentCapabilities.promptCapabilities.image, true);
  assert.equal(init.agentInfo.name, 'opencode-acp-adapter');
});

test('session/new creates the session on the shared serve with cwd as directory', async (t) => {
  const { stub, acp } = await setup(t, textEvents());
  const result = await acp.request('session/new', { cwd: '/home/user/code', mcpServers: [] });
  assert.equal(result.sessionId, SESSION_ID);
  const create = stub.calls.find((c) => c.method === 'POST' && c.pathname === '/session');
  assert.ok(create, 'stub serve saw POST /session');
  assert.equal(create.query.get('directory'), '/home/user/code');
});

test('session/prompt streams serve SSE parts as session/update and completes on idle', async (t) => {
  const { stub, acp } = await setup(t, textEvents());
  await acp.request('session/new', { cwd: '/home/user/code', mcpServers: [] });
  const result = await acp.request('session/prompt', {
    sessionId: SESSION_ID,
    prompt: [{ type: 'text', text: 'say hello' }],
  });
  assert.equal(result.stopReason, 'end_turn');
  const promptCall = stub.calls.find((c) => c.pathname.endsWith('/prompt_async'));
  assert.ok(promptCall, 'stub serve saw prompt_async');
  assert.deepEqual(promptCall.body.parts, [{ type: 'text', text: 'say hello' }]);
  const chunks = acp.notifications
    .filter((n) => n.method === 'session/update' && n.params.update.sessionUpdate === 'agent_message_chunk')
    .map((n) => n.params.update.content.text);
  assert.equal(chunks.join(''), 'Hello world');
});

test('serve permission events round-trip through session/request_permission', async (t) => {
  const script = (emit) => {
    emit({
      type: 'permission.updated',
      properties: { id: 'perm1', sessionID: SESSION_ID, type: 'bash', title: 'Run ls', metadata: {} },
    });
    // Idle arrives only after the adapter posts the permission reply;
    // the stub cannot see that from here, so idle on a delay and let
    // the assertions below order-check via the recorded calls.
    setTimeout(() => emit({ type: 'session.idle', properties: { sessionID: SESSION_ID } }), 500);
  };
  const { stub, acp } = await setup(t, script);
  await acp.request('session/new', { cwd: '/home/user/code', mcpServers: [] });
  const promptPromise = acp.request('session/prompt', {
    sessionId: SESSION_ID,
    prompt: [{ type: 'text', text: 'list files' }],
  });
  const permRequest = await acp.waitFor(
    () => acp.clientRequests.find((r) => r.method === 'session/request_permission'),
    'session/request_permission',
  );
  assert.equal(permRequest.params.sessionId, SESSION_ID);
  assert.equal(permRequest.params.toolCall.toolCallId, 'perm1');
  acp.respond(permRequest.id, { outcome: { outcome: 'selected', optionId: 'allow_once' } });
  const result = await promptPromise;
  assert.equal(result.stopReason, 'end_turn');
  const permCall = stub.calls.find((c) => c.pathname === `/session/${SESSION_ID}/permissions/perm1`);
  assert.ok(permCall, 'stub serve saw POST permissions/perm1');
  assert.deepEqual(permCall.body, { response: 'once' });
});

test('session/cancel aborts on the serve and the prompt resolves cancelled', async (t) => {
  const { stub, acp } = await setup(t, () => {
    // Never idles on its own: only the abort (stub emits idle) ends it.
  });
  await acp.request('session/new', { cwd: '/home/user/code', mcpServers: [] });
  const promptPromise = acp.request('session/prompt', {
    sessionId: SESSION_ID,
    prompt: [{ type: 'text', text: 'long task' }],
  });
  await acp.waitFor(
    () => stub.calls.find((c) => c.pathname.endsWith('/prompt_async')),
    'prompt_async to reach the stub',
  );
  acp.notify('session/cancel', { sessionId: SESSION_ID });
  const result = await promptPromise;
  assert.equal(result.stopReason, 'cancelled');
  assert.ok(
    stub.calls.find((c) => c.pathname === `/session/${SESSION_ID}/abort`),
    'stub serve saw POST abort',
  );
});
