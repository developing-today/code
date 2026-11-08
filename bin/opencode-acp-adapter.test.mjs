// Tests for the OpenCode ACP-to-HTTP adapter (#1243).
// Run: node --test bin/opencode-acp-adapter.test.mjs
//
// A scripted ACP client (this file) drives the adapter over stdio
// against a stub OpenCode serve HTTP API (v2 contract, #4614 — shapes
// probed live from the standing server, task 6571), covering:
// initialize, session/new -> POST /api/session (cwd as
// location.directory, {data} envelope), a streamed prompt round-trip
// (POST /api/session/:id/prompt {text} + GET /api/event SSE
// session.text.delta -> session/update notifications, completion on
// session.execution.succeeded), a permission round-trip
// (permission.asked -> session/request_permission -> POST
// /api/session/:id/permission/:id/reply {decision}), and cancel
// (session/cancel -> POST /api/session/:id/interrupt, prompt resolves
// "cancelled").
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtempSync, mkdirSync, writeFileSync } from 'node:fs';
import http from 'node:http';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const binDir = path.dirname(fileURLToPath(import.meta.url));
const SESSION_ID = 'ses_test1243';

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

// ---------------------------------------------------------------------------
// Stub serve API. `script` runs when the v2 prompt call arrives; it gets
// `emit(event)` to push SSE events to the adapter.
// ---------------------------------------------------------------------------
function startStubServe(script) {
  const calls = [];
  const sseClients = new Set();
  let sessionStatus = {};
  let statusResponseDelay = 0;
  let onStatusRequest = null;
  let promptAsyncResponseDelay = 0;
  const emit = (event) => {
    const payload = `data: ${JSON.stringify(event)}\n\n`;
    for (const res of sseClients) res.write(payload);
  };
  const serveControls = {
    setStatus: (status) => {
      sessionStatus = status;
    },
    setStatusResponseDelay: (ms) => {
      statusResponseDelay = ms;
    },
    onStatusRequest: (handler) => {
      onStatusRequest = handler;
    },
    setPromptAsyncDelay: (ms) => {
      promptAsyncResponseDelay = ms;
    },
    dropEventStreams: () => {
      for (const res of sseClients) res.destroy();
    },
  };
  const server = http.createServer((req, res) => {
    const url = new URL(req.url, 'http://127.0.0.1');
    let raw = '';
    req.on('data', (chunk) => {
      raw += chunk;
    });
    req.on('end', () => {
      const body = raw ? JSON.parse(raw) : null;
      calls.push({
        method: req.method,
        pathname: url.pathname,
        query: url.searchParams,
        body,
        authorization: req.headers.authorization ?? null,
      });
      if (req.method === 'GET' && url.pathname === '/api/event') {
        res.writeHead(200, { 'Content-Type': 'text/event-stream', 'Cache-Control': 'no-cache' });
        res.flushHeaders();
        sseClients.add(res);
        // NB: req 'close' fires as soon as the (bodyless) request is
        // consumed, long before the stream ends — unregister on the
        // response closing instead.
        res.on('close', () => sseClients.delete(res));
        return;
      }
      if (req.method === 'GET' && url.pathname === '/api/session/active') {
        // v2 active set, driven by the setStatus control: sessions
        // whose status type is busy/retry are the active ones.
        const statusSnapshot = sessionStatus;
        onStatusRequest?.();
        const sendActive = () => {
          const activeMap = {};
          for (const [id, s] of Object.entries(statusSnapshot)) {
            if (s && (s.type === 'busy' || s.type === 'retry')) activeMap[id] = s;
          }
          res.writeHead(200, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({ data: activeMap }));
        };
        if (statusResponseDelay) setTimeout(sendActive, statusResponseDelay);
        else sendActive();
        return;
      }
      if (req.method === 'POST' && url.pathname === '/api/session') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ data: { id: SESSION_ID } }));
        return;
      }
      if (req.method === 'POST' && url.pathname === `/api/session/${SESSION_ID}/prompt`) {
        const respondPrompt = () => {
          res.writeHead(200, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({ data: { id: 'msg_test1', sessionID: SESSION_ID, type: 'user' } }));
          setImmediate(() => script(emit, body, serveControls));
        };
        if (promptAsyncResponseDelay) setTimeout(respondPrompt, promptAsyncResponseDelay);
        else respondPrompt();
        return;
      }
      if (req.method === 'POST' && /\/permission\/.+\/reply$/.test(url.pathname)) {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ data: true }));
        return;
      }
      if (req.method === 'POST' && url.pathname === `/api/session/${SESSION_ID}/interrupt`) {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ data: true }));
        emit({ type: 'session.execution.succeeded', data: { sessionID: SESSION_ID } });
        return;
      }
      res.writeHead(404, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ error: 'not found' }));
    });
  });
  return new Promise((resolve) => {
    server.listen(0, '127.0.0.1', () => {
      resolve({ server, calls, port: server.address().port, controls: serveControls });
    });
  });
}

// ---------------------------------------------------------------------------
// Scripted ACP client over the adapter's stdio.
// ---------------------------------------------------------------------------
function startAdapter(serverPort, env = {}) {
  const child = spawn(
    process.execPath,
    [path.join(binDir, 'opencode-acp-adapter.mjs'), '--server', `http://127.0.0.1:${serverPort}`],
    {
      env: {
        ...process.env,
        // Hermetic credential resolution: never the developer's real
        // env password or real state file unless a test opts in.
        OPENCODE_SERVER_PASSWORD: '',
        OPENCODE_AUTH_FILE: path.join(binDir, 'nonexistent-auth-fixture.json'),
        ...env,
      },
    },
  );
  let stderr = '';
  child.stderr.setEncoding('utf8');
  child.stderr.on('data', (chunk) => {
    stderr += chunk;
  });
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
  return { child, request, notify, respond, notifications, clientRequests, waitFor, stderr: () => stderr };
}

async function setup(t, script, env) {
  const stub = await startStubServe(script);
  const acp = startAdapter(stub.port, env);
  t.after(() => {
    acp.child.kill('SIGKILL');
    stub.server.close();
  });
  const init = await acp.request('initialize', { protocolVersion: 1, clientCapabilities: {} });
  return { stub, acp, init };
}

const textEvents = () => (emit) => {
  emit({ type: 'session.text.delta', data: { sessionID: SESSION_ID, delta: 'Hello ' } });
  emit({ type: 'session.text.delta', data: { sessionID: SESSION_ID, delta: 'world' } });
  emit({ type: 'session.execution.succeeded', data: { sessionID: SESSION_ID } });
};

test('initialize negotiates ACP v1 and advertises prompt capabilities', async (t) => {
  const { init } = await setup(t, textEvents());
  assert.equal(init.protocolVersion, 1);
  assert.equal(init.agentCapabilities.loadSession, false);
  assert.equal(init.agentCapabilities.promptCapabilities.image, true);
  assert.equal(init.agentInfo.name, 'opencode-acp-adapter');
});

test('a JSON null line on stdin is ignored and the adapter keeps answering (#6919)', async (t) => {
  const { acp } = await setup(t, textEvents());
  // `null` is valid JSON but not a message object; before the fix its
  // destructure in dispatch threw and the process exited (code 1), so
  // every later request went unanswered. Other non-object JSON values
  // (42, "hi", true) were already harmless no-ops.
  acp.child.stdin.write('null\n');
  acp.child.stdin.write('42\n');
  await sleep(150);
  assert.equal(acp.child.exitCode, null, 'adapter still running after a null line');
  const again = await Promise.race([
    acp.request('initialize', { protocolVersion: 1, clientCapabilities: {} }),
    sleep(3000).then(() => {
      throw new Error('adapter did not answer initialize after a null line');
    }),
  ]);
  assert.equal(again.protocolVersion, 1);
  assert.equal(again.agentInfo.name, 'opencode-acp-adapter');
});

test('session/new creates the session on the shared serve with cwd as directory', async (t) => {
  const { stub, acp } = await setup(t, textEvents());
  const result = await acp.request('session/new', { cwd: '/home/user/code', mcpServers: [] });
  assert.equal(result.sessionId, SESSION_ID);
  const create = stub.calls.find((c) => c.method === 'POST' && c.pathname === '/api/session');
  assert.ok(create, 'stub serve saw POST /api/session');
  assert.deepEqual(create.body, { location: { directory: '/home/user/code' } });
});

test('session/prompt streams serve SSE deltas as session/update and completes on execution.succeeded', async (t) => {
  const { stub, acp } = await setup(t, textEvents());
  await acp.request('session/new', { cwd: '/home/user/code', mcpServers: [] });
  const result = await acp.request('session/prompt', {
    sessionId: SESSION_ID,
    prompt: [{ type: 'text', text: 'say hello' }],
  });
  assert.equal(result.stopReason, 'end_turn');
  const promptCall = stub.calls.find((c) => c.pathname === `/api/session/${SESSION_ID}/prompt`);
  assert.ok(promptCall, 'stub serve saw POST /api/session/:id/prompt');
  assert.deepEqual(promptCall.body, { text: 'say hello' });
  const chunks = acp.notifications
    .filter((n) => n.method === 'session/update' && n.params.update.sessionUpdate === 'agent_message_chunk')
    .map((n) => n.params.update.content.text);
  assert.equal(chunks.join(''), 'Hello world');
});

test('serve permission events round-trip through session/request_permission', async (t) => {
  const script = (emit) => {
    emit({
      type: 'permission.asked',
      data: { id: 'perm1', sessionID: SESSION_ID, action: 'bash', resources: ['ls'], metadata: {} },
    });
    // Completion arrives only after the adapter posts the permission
    // reply; the stub cannot see that from here, so succeed on a delay
    // and let the assertions below order-check via the recorded calls.
    setTimeout(() => emit({ type: 'session.execution.succeeded', data: { sessionID: SESSION_ID } }), 500);
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
  const permCall = stub.calls.find(
    (c) => c.pathname === `/api/session/${SESSION_ID}/permission/perm1/reply`,
  );
  assert.ok(permCall, 'stub serve saw POST /api/session/:id/permission/perm1/reply');
  assert.deepEqual(permCall.body, { decision: 'once' });
});

test('session/cancel interrupts on the serve and the prompt resolves cancelled', async (t) => {
  const { stub, acp } = await setup(t, () => {
    // Never completes on its own: only the interrupt (stub emits
    // execution.succeeded) ends it.
  });
  await acp.request('session/new', { cwd: '/home/user/code', mcpServers: [] });
  const promptPromise = acp.request('session/prompt', {
    sessionId: SESSION_ID,
    prompt: [{ type: 'text', text: 'long task' }],
  });
  await acp.waitFor(
    () => stub.calls.find((c) => c.pathname === `/api/session/${SESSION_ID}/prompt`),
    'v2 prompt call to reach the stub',
  );
  acp.notify('session/cancel', { sessionId: SESSION_ID });
  const result = await promptPromise;
  assert.equal(result.stopReason, 'cancelled');
  assert.ok(
    stub.calls.find((c) => c.pathname === `/api/session/${SESSION_ID}/interrupt`),
    'stub serve saw POST interrupt',
  );
});

test('prompt completed during an event-stream drop is reconciled on reconnect (#4617)', async (t) => {
  let promptCount = 0;
  const script = (emit, body, serve) => {
    promptCount += 1;
    if (promptCount === 1) {
      serve.setStatus({ [SESSION_ID]: { type: 'busy' } });
      // The stream drops mid-prompt and the server finishes the prompt
      // during the gap: no completion event ever reaches the adapter, and
      // the session leaves the active set. Without reconciliation the
      // ACP request would hang forever.
      setTimeout(() => {
        serve.dropEventStreams();
        serve.setStatus({});
      }, 50);
      return;
    }
    emit({ type: 'session.execution.succeeded', data: { sessionID: SESSION_ID } });
  };
  const { stub, acp } = await setup(t, script);
  await acp.request('session/new', { cwd: '/home/user/code', mcpServers: [] });
  const result = await acp.request('session/prompt', {
    sessionId: SESSION_ID,
    prompt: [{ type: 'text', text: 'long task' }],
  });
  assert.equal(result.stopReason, 'end_turn');
  assert.ok(
    stub.calls.find((c) => c.method === 'GET' && c.pathname === '/api/session/active'),
    'adapter queried /api/session/active after reconnecting',
  );
  // The session is not wedged: a later prompt is accepted and completes.
  const second = await acp.request('session/prompt', {
    sessionId: SESSION_ID,
    prompt: [{ type: 'text', text: 'again' }],
  });
  assert.equal(second.stopReason, 'end_turn');
});

test('reconciliation does not finish a newer prompt using a stale active-set snapshot', async (t) => {
  let promptCount = 0;
  let secondPromptIdleEmitted = false;
  const script = (emit, body, serve) => {
    promptCount += 1;
    if (promptCount === 1) {
      serve.setStatus({});
      serve.setStatusResponseDelay(500);
      // Deliver the first prompt's completion over the reconnected stream
      // while the active-set response is delayed. The client will start another
      // prompt before the stale snapshot arrives.
      serve.onStatusRequest(() => {
        emit({ type: 'session.execution.succeeded', data: { sessionID: SESSION_ID } });
      });
      setTimeout(() => serve.dropEventStreams(), 50);
      return;
    }
    serve.setStatus({ [SESSION_ID]: { type: 'busy' } });
    setTimeout(() => {
      secondPromptIdleEmitted = true;
      serve.setStatus({});
      emit({ type: 'session.execution.succeeded', data: { sessionID: SESSION_ID } });
    }, 1000);
  };
  const { acp } = await setup(t, script);
  await acp.request('session/new', { cwd: '/home/user/code', mcpServers: [] });

  const first = await acp.request('session/prompt', {
    sessionId: SESSION_ID,
    prompt: [{ type: 'text', text: 'first task' }],
  });
  assert.equal(first.stopReason, 'end_turn');

  const second = await acp.request('session/prompt', {
    sessionId: SESSION_ID,
    prompt: [{ type: 'text', text: 'second task' }],
  });
  assert.equal(second.stopReason, 'end_turn');
  assert.equal(secondPromptIdleEmitted, true, 'new prompt waited for its own completion event');
});

test('reconciliation does not finish a prompt whose submission is still in flight', async (t) => {
  // The prompt POST stays in flight while the stream drops and
  // reconnects. The active-set snapshot taken during reconciliation predates
  // server-side execution (idle/absent), so without the submitted guard
  // the prompt would resolve end_turn early and its later streamed output
  // would be discarded as orphan events.
  const script = (emit) => {
    emit({ type: 'session.text.delta', data: { sessionID: SESSION_ID, delta: 'late output' } });
    emit({ type: 'session.execution.succeeded', data: { sessionID: SESSION_ID } });
  };
  const { stub, acp } = await setup(t, script);
  await acp.request('session/new', { cwd: '/home/user/code', mcpServers: [] });
  stub.controls.setPromptAsyncDelay(2500);
  const promptPromise = acp.request('session/prompt', {
    sessionId: SESSION_ID,
    prompt: [{ type: 'text', text: 'slow submission' }],
  });
  // The prompt entry is registered as soon as the POST reaches the stub,
  // while the prompt response (submission) is still delayed.
  await acp.waitFor(
    () => stub.calls.find((c) => c.pathname.endsWith('/prompt')),
    'prompt POST to reach the stub',
  );
  // Drop the stream mid-submission with the session absent from the active set:
  // reconnect reconciliation must leave the unsubmitted entry alone.
  stub.controls.setStatus({});
  stub.controls.dropEventStreams();
  const result = await promptPromise;
  assert.equal(result.stopReason, 'end_turn');
  const chunks = acp.notifications
    .filter((n) => n.method === 'session/update' && n.params.update.sessionUpdate === 'agent_message_chunk')
    .map((n) => n.params.update.content.text);
  assert.equal(chunks.join(''), 'late output', 'streamed output after reconcile was not discarded');
});

test('a cancel arriving after a finished prompt does not mark the next prompt cancelled (#4618)', async (t) => {
  const { acp } = await setup(t, textEvents());
  await acp.request('session/new', { cwd: '/home/user/code', mcpServers: [] });
  const first = await acp.request('session/prompt', {
    sessionId: SESSION_ID,
    prompt: [{ type: 'text', text: 'first' }],
  });
  assert.equal(first.stopReason, 'end_turn');
  acp.notify('session/cancel', { sessionId: SESSION_ID });
  // Let the stray cancel handler run against a completed prompt before the
  // next prompt starts; the regression is deterministic without a pause.
  await sleep(50);
  const second = await acp.request('session/prompt', {
    sessionId: SESSION_ID,
    prompt: [{ type: 'text', text: 'second' }],
  });
  assert.equal(second.stopReason, 'end_turn');
});

// ---------------------------------------------------------------------------
// #4615: the configured gateway launch path (acpx spawns the adapter
// with no OPENCODE_SERVER_PASSWORD) must still authenticate, by
// runtime-loading the standing server's own state file. Fixtures use
// synthetic passwords only.
// ---------------------------------------------------------------------------

const basicFor = (password) => `Basic ${Buffer.from(`opencode:${password}`).toString('base64')}`;

test('gateway launch path: adapter loads the credential from the default state file under HOME', async (t) => {
  const home = mkdtempSync(path.join(tmpdir(), 'acp-adapter-home-'));
  const stateDir = path.join(home, '.local', 'state', 'opencode');
  mkdirSync(stateDir, { recursive: true });
  writeFileSync(path.join(stateDir, 'service-prod.json'), JSON.stringify({ password: 'synthetic-gateway-password' }));
  const { stub, acp } = await setup(t, textEvents(), {
    HOME: home,
    OPENCODE_AUTH_FILE: '',
  });
  await acp.request('session/new', { cwd: '/home/user/code', mcpServers: [] });
  const create = stub.calls.find((c) => c.method === 'POST' && c.pathname === '/api/session');
  assert.equal(create.authorization, basicFor('synthetic-gateway-password'));
  assert.ok(!acp.stderr().includes('synthetic-gateway-password'), 'credential never reaches stderr');
});

test('OPENCODE_AUTH_FILE fixture authenticates; the env password wins over the file', async (t) => {
  const dir = mkdtempSync(path.join(tmpdir(), 'acp-adapter-auth-'));
  const authFile = path.join(dir, 'service-prod.json');
  writeFileSync(authFile, JSON.stringify({ password: 'synthetic-file-password' }));

  const fromFile = await setup(t, textEvents(), { OPENCODE_AUTH_FILE: authFile });
  await fromFile.acp.request('session/new', { cwd: '/home/user/code', mcpServers: [] });
  const fileCreate = fromFile.stub.calls.find((c) => c.method === 'POST' && c.pathname === '/api/session');
  assert.equal(fileCreate.authorization, basicFor('synthetic-file-password'));

  const fromEnv = await setup(t, textEvents(), {
    OPENCODE_AUTH_FILE: authFile,
    OPENCODE_SERVER_PASSWORD: 'synthetic-env-password',
  });
  await fromEnv.acp.request('session/new', { cwd: '/home/user/code', mcpServers: [] });
  const envCreate = fromEnv.stub.calls.find((c) => c.method === 'POST' && c.pathname === '/api/session');
  assert.equal(envCreate.authorization, basicFor('synthetic-env-password'));
});

test('malformed credential state file warns on stderr and sends no auth header', async (t) => {
  const dir = mkdtempSync(path.join(tmpdir(), 'acp-adapter-bad-'));
  const authFile = path.join(dir, 'service-prod.json');
  writeFileSync(authFile, 'this is not json');
  const { stub, acp } = await setup(t, textEvents(), { OPENCODE_AUTH_FILE: authFile });
  const result = await acp.request('session/new', { cwd: '/home/user/code', mcpServers: [] });
  assert.equal(result.sessionId, SESSION_ID);
  const create = stub.calls.find((c) => c.method === 'POST' && c.pathname === '/api/session');
  assert.equal(create.authorization, null);
  await acp.waitFor(() => acp.stderr().includes('not valid JSON') || null, 'stderr warning');
  assert.ok(acp.stderr().includes(authFile), 'warning names the state file');
});
