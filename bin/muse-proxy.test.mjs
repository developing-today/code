// Tests for muse-proxy's streaming/terminal-text reconciliation (#484)
// and its warm serve/MSP session path (#480). The exec-path tests pin
// MUSE_PROXY_WARM=0; the warm tests drive a stub `muse serve` host.
// The flag-probe tests drive the stub's canned --help surfaces
// (1.4.2-shaped by default, 1.4.13-shaped with STUB_HELP_ALL_FLAGS=1).
// Run: node --test bin/muse-proxy.test.mjs
//
// The proxy spawns one `muse exec --json` child per request. These tests
// put a stub `muse` first on PATH (and point HOME at an empty dir so the
// real ~/.local/bin/muse is not preferred) whose JSONL output is fixed:
// the deltas are a strict prefix of the terminal event's authoritative
// text — the case where streaming used to silently drop the tail.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import http from 'node:http';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const binDir = path.dirname(fileURLToPath(import.meta.url));

const STUB = `#!/usr/bin/env node
// Stub muse: 'exec' reproduces the fixed JSONL behaviours the exec-path
// tests assert on; 'serve' speaks just enough MSP (NDJSON JSON-RPC) for
// the warm-path tests, logging session/turn traffic to $STUB_LOG.
const fs = require('fs');
const args = process.argv.slice(2);
if (process.env.STUB_LOG) fs.appendFileSync(process.env.STUB_LOG, 'ARGV ' + args.join(' ') + '\\n');
if (args.includes('--help')) {
  // Canned --help for the proxy's flag-capability probe. By default the
  // stub advertises the 1.4.2-shaped surface (no serve
  // --disable-request-user-input, no exec --disable-reminders);
  // STUB_HELP_ALL_FLAGS=1 advertises the 1.4.13-shaped surface instead.
  const all = process.env.STUB_HELP_ALL_FLAGS === '1';
  if (args[0] === 'serve') {
    console.log('Usage: muse serve [--provider <PROVIDER>] [--model <MODEL>] [OPTIONS]');
    console.log('      --provider <PROVIDER>');
    console.log('      --model <MODEL>');
    console.log('      --no-session-log');
    if (all) console.log('      --disable-request-user-input');
  } else {
    console.log('Usage: muse exec [OPTIONS] [PROMPT]');
    console.log('      --json');
    console.log('      --prompt-file <PATH>');
    console.log('      --model <ID>');
    console.log('      --reasoning-effort <EFFORT>');
    if (all) console.log('      --disable-reminders');
    console.log('      --user-input-auto-resolve');
    console.log('      --approval-mode <MODE>');
  }
  process.exit(0);
}
if (args[0] === 'serve') {
  if (process.env.STUB_SERVE_FAIL === '1') process.exit(3);
  const log = (line) => { if (process.env.STUB_LOG) fs.appendFileSync(process.env.STUB_LOG, line + '\\n'); };
  let buf = '';
  process.stdin.on('data', (d) => {
    buf += d.toString('utf-8');
    let i;
    while ((i = buf.indexOf('\\n')) >= 0) {
      const line = buf.slice(0, i); buf = buf.slice(i + 1);
      if (!line.trim()) continue;
      let msg; try { msg = JSON.parse(line); } catch { continue; }
      if (msg.id === undefined || !msg.method) continue;
      const reply = (result) => process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: msg.id, result }) + '\\n');
      const notify = (method, params) => process.stdout.write(JSON.stringify({ jsonrpc: '2.0', method, params }) + '\\n');
      if (msg.method === 'initialize') {
        reply({ serverInfo: { name: 'stub-muse', version: '0' } });
      } else if (msg.method === 'session/start') {
        log('START ' + msg.params.modelId);
        reply({ session: { sessionId: 'stub-session-1' } });
      } else if (msg.method === 'session/resume') {
        log('RESUME ' + msg.params.sessionId);
        reply({ session: { sessionId: msg.params.sessionId } });
      } else if (msg.method === 'turn/start') {
        const text = (msg.params.input || []).map(p => p.text || '').join('');
        log('TURN ' + text);
        const sid = msg.params.sessionId;
        const turnId = msg.params.commandId;
        reply({ commandId: msg.params.commandId, turnId, disposition: 'started', startedNewTurn: true, status: 'accepted' });
        notify('item/started', { sessionId: sid, item: { itemId: 'a1', kind: 'agentMessage' } });
        notify('item/delta', { sessionId: sid, itemId: 'a1', delta: 'RE:' + text });
        notify('item/completed', { sessionId: sid, item: { itemId: 'a1', kind: 'agentMessage', messagePhase: 'finalAnswer', text: 'RE:' + text } });
        notify('session/tokenUsage', { sessionId: sid, turnId, usage: { inputTokens: 42, outputTokens: 7, cacheReadTokens: 40 } });
        notify('turn/completed', { sessionId: sid, turnId, terminal: 'completed' });
      } else {
        reply({});
      }
    }
  });
} else {
  let promptFile = null;
  for (let i = 0; i < args.length; i++) if (args[i] === '--prompt-file') promptFile = args[i + 1];
  let prompt = '';
  try { prompt = fs.readFileSync(promptFile, 'utf-8'); } catch {}
  const line = (o) => console.log(JSON.stringify(o));
  if (prompt.includes('FAILME')) { console.error('stub muse: auth expired'); process.exit(3); }
  if (prompt.includes('FAILPARTIAL')) {
    line({ payload_type: 'run.output.delta', payload: { text: 'partial answer' } });
    line({ payload_type: 'run.terminal.failed', payload: { terminal: 'failed', text: 'partial answer' } });
    console.error('stub muse: run failed midway');
    process.exit(0);
  }
  if (prompt.includes('CRASHPARTIAL')) {
    line({ payload_type: 'run.output.delta', payload: { text: 'partial answer' } });
    console.error('stub muse: crashed midway');
    process.exit(3);
  }
  if (prompt.includes('FAILSTATE')) {
    line({ payload_type: 'run.terminal.failed', payload: { terminal: 'failed' } });
    process.exit(0);
  }
  if (prompt.includes('SIGKILLME')) { process.kill(process.pid, 'SIGKILL'); }
  if (prompt.includes('DIVERGE')) {
    line({ payload_type: 'run.output.delta', payload: { text: 'abc' } });
    line({ payload_type: 'run.terminal.completed', payload: { terminal: 'completed', text: 'xyz-longer' } });
  } else {
    line({ payload_type: 'run.output.delta', payload: { text: 'Hel' } });
    line({ payload_type: 'run.usage', payload: { models: [{ input_tokens: 100, output_tokens: 3 }] } });
    line({ payload_type: 'run.terminal.completed', payload: { terminal: 'completed', text: 'Hello world' } });
  }
}`;

const freePort = () => new Promise((resolve, reject) => {
  const srv = net.createServer();
  srv.once('error', reject);
  srv.listen(0, '127.0.0.1', () => {
    const { port } = srv.address();
    srv.close(() => resolve(port));
  });
});

const startProxy = async (extraEnv = {}) => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'muse-proxy-test-'));
  const stubDir = path.join(tmp, 'stub');
  fs.mkdirSync(stubDir);
  fs.mkdirSync(path.join(tmp, 'home'));
  fs.writeFileSync(path.join(stubDir, 'muse'), STUB, { mode: 0o755 });
  // The proxy prefers $HOME/.local/bin/muse over PATH, and on NixOS hosts
  // /run/current-system/sw/bin/muse exists and would shadow the stub with
  // the real CLI. Plant the stub at the preferred path too so the harness
  // is hermetic on those hosts.
  fs.mkdirSync(path.join(tmp, 'home', '.local', 'bin'), { recursive: true });
  fs.writeFileSync(path.join(tmp, 'home', '.local', 'bin', 'muse'), STUB, { mode: 0o755 });
  const port = await freePort();
  const logFile = path.join(tmp, 'stub.log');
  const child = spawn(process.execPath, [path.join(binDir, 'muse-proxy.mjs')], {
    env: {
      ...process.env,
      HOME: path.join(tmp, 'home'),
      PATH: `${stubDir}:${process.env.PATH}`,
      MUSE_PROXY_PORT: String(port),
      MUSE_PROXY_WORKDIR: path.join(tmp, 'work'),
      MUSE_PROXY_WARM: '0',
      STUB_LOG: logFile,
      ...extraEnv
    },
    stdio: ['ignore', 'pipe', 'pipe']
  });
  let log = '';
  child.stdout.on('data', d => { log += d; });
  child.stderr.on('data', d => { log += d; });
  const deadline = Date.now() + 10000;
  for (;;) {
    try {
      const res = await fetch(`http://127.0.0.1:${port}/health`);
      if (res.ok) break;
    } catch { /* not up yet */ }
    if (Date.now() > deadline) throw new Error(`proxy did not start:\n${log}`);
    await new Promise(r => setTimeout(r, 50));
  }
  return { port, stop: () => child.kill('SIGKILL'), logFile, home: path.join(tmp, 'home') };
};

const chat = async (port, body) => {
  const res = await fetch(`http://127.0.0.1:${port}/v1/chat/completions`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body)
  });
  assert.equal(res.status, 200);
  return res.text();
};

const streamedContent = (sse) => {
  let content = '';
  let finish = null;
  for (const line of sse.split('\n')) {
    if (!line.startsWith('data: ') || line === 'data: [DONE]') continue;
    const chunk = JSON.parse(line.slice('data: '.length));
    const choice = chunk.choices[0];
    if (choice.delta && typeof choice.delta.content === 'string') content += choice.delta.content;
    if (choice.finish_reason) finish = choice.finish_reason;
  }
  return { content, finish };
};

test('streaming emits the terminal text tail the deltas missed (#484)', async (t) => {
  const { port, stop } = await startProxy();
  t.after(stop);
  const messages = [{ role: 'user', content: 'hi' }];

  const plain = JSON.parse(await chat(port, { messages }));
  assert.equal(plain.choices[0].message.content, 'Hello world');

  const { content, finish } = streamedContent(await chat(port, { messages, stream: true }));
  assert.equal(content, 'Hello world', 'streamed content must equal the non-streamed content');
  assert.equal(finish, 'stop');
});

test('execution failure: stream gets an SSE-framed error + [DONE], non-stream gets 500 (#670)', async (t) => {
  const { port, stop } = await startProxy();
  t.after(stop);
  const messages = [{ role: 'user', content: 'please FAILME now' }];

  const plainRes = await fetch(`http://127.0.0.1:${port}/v1/chat/completions`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ messages })
  });
  assert.equal(plainRes.status, 500);
  const plainBody = JSON.parse(await plainRes.text());
  assert.match(plainBody.error.message, /Muse execution failed: stub muse: auth expired/);

  const streamRes = await fetch(`http://127.0.0.1:${port}/v1/chat/completions`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ messages, stream: true })
  });
  assert.equal(streamRes.status, 200);
  assert.match(streamRes.headers.get('content-type'), /text\/event-stream/);
  const sse = await streamRes.text();
  const dataLines = sse.split('\n').filter(l => l.startsWith('data: '));
  assert.ok(dataLines.length >= 2, `expected a framed error event and [DONE], got: ${JSON.stringify(sse)}`);
  const errorEvent = JSON.parse(dataLines[0].slice('data: '.length));
  assert.match(errorEvent.error.message, /Muse execution failed: stub muse: auth expired/);
  assert.equal(dataLines[dataLines.length - 1], 'data: [DONE]');
});

test('partial output then failure: masked success becomes an error in both modes (#672)', async (t) => {
  const { port, stop } = await startProxy();
  t.after(stop);

  for (const marker of ['FAILPARTIAL', 'CRASHPARTIAL']) {
    const messages = [{ role: 'user', content: `please ${marker} now` }];

    const plainRes = await fetch(`http://127.0.0.1:${port}/v1/chat/completions`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ messages })
    });
    assert.equal(plainRes.status, 500, `${marker}: non-stream must be a 500, not a 200 'stop'`);
    const plainBody = JSON.parse(await plainRes.text());
    assert.match(plainBody.error.message, /Muse execution failed: stub muse:/);

    const streamRes = await fetch(`http://127.0.0.1:${port}/v1/chat/completions`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ messages, stream: true })
    });
    assert.equal(streamRes.status, 200);
    const sse = await streamRes.text();
    const dataLines = sse.split('\n').filter(l => l.startsWith('data: '));
    assert.equal(dataLines[dataLines.length - 1], 'data: [DONE]', `${marker}: stream must end with [DONE]`);
    const events = dataLines.slice(0, -1).map(l => JSON.parse(l.slice('data: '.length)));
    const errorEvent = events.find(e => e.error);
    assert.ok(errorEvent, `${marker}: stream must carry an error event, got: ${JSON.stringify(sse)}`);
    assert.match(errorEvent.error.message, /Muse execution failed: stub muse:/);
    const stopChunk = events.find(e => e.choices && e.choices[0].finish_reason === 'stop');
    assert.equal(stopChunk, undefined, `${marker}: no chunk may claim finish_reason 'stop'`);
  }
});

const assertEmptyStderrFallback = async (t, marker, detail) => {
  const { port, stop } = await startProxy();
  t.after(stop);

  {
    const messages = [{ role: 'user', content: `please ${marker} now` }];

    const plainRes = await fetch(`http://127.0.0.1:${port}/v1/chat/completions`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ messages })
    });
    assert.equal(plainRes.status, 500, `${marker}: non-stream must be a 500`);
    const plainBody = JSON.parse(await plainRes.text());
    assert.match(plainBody.error.message, detail, `${marker}: non-stream detail must name the cause, got: ${plainBody.error.message}`);
    assert.doesNotMatch(plainBody.error.message, /code (null|0)\b/);

    const streamRes = await fetch(`http://127.0.0.1:${port}/v1/chat/completions`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ messages, stream: true })
    });
    assert.equal(streamRes.status, 200);
    const sse = await streamRes.text();
    const dataLines = sse.split('\n').filter(l => l.startsWith('data: '));
    assert.equal(dataLines[dataLines.length - 1], 'data: [DONE]', `${marker}: stream must end with [DONE]`);
    const events = dataLines.slice(0, -1).map(l => JSON.parse(l.slice('data: '.length)));
    const errorEvent = events.find(e => e.error);
    assert.ok(errorEvent, `${marker}: stream must carry an error event, got: ${JSON.stringify(sse)}`);
    assert.match(errorEvent.error.message, detail, `${marker}: stream detail must name the cause, got: ${errorEvent.error.message}`);
    assert.doesNotMatch(errorEvent.error.message, /code (null|0)\b/);
  }
};

test('empty-stderr fallback names the terminal state (#674)', async (t) => {
  await assertEmptyStderrFallback(t, 'FAILSTATE', /Muse execution failed: muse run terminated with state 'failed'/);
});

test('empty-stderr fallback names the killing signal, not code null (#674)', async (t) => {
  await assertEmptyStderrFallback(t, 'SIGKILLME', /Muse execution failed: muse exec terminated by signal SIGKILL/);
});

test('divergent terminal text: non-stream uses it, stream keeps the deltas already sent', async (t) => {
  const { port, stop } = await startProxy();
  t.after(stop);
  const messages = [{ role: 'user', content: 'please DIVERGE now' }];

  const plain = JSON.parse(await chat(port, { messages }));
  assert.equal(plain.choices[0].message.content, 'xyz-longer');

  const { content, finish } = streamedContent(await chat(port, { messages, stream: true }));
  assert.equal(content, 'abc');
  assert.equal(finish, 'stop');
});

test('warm serve path: second turn sends only the new user message (#480)', async (t) => {
  const { port, stop, logFile } = await startProxy({ MUSE_PROXY_WARM: '1' });
  t.after(stop);

  const turn1 = JSON.parse(await chat(port, {
    model: 'muse-spark-1.3',
    messages: [{ role: 'user', content: 'alpha one' }]
  }));
  assert.equal(turn1.choices[0].message.content, 'RE:alpha one');
  assert.equal(turn1.usage.prompt_tokens, 42, 'usage must come from the MSP tokenUsage notification');
  assert.equal(turn1.usage.prompt_tokens_details.cached_tokens, 40, 'MSP cacheReadTokens must surface as prompt_tokens_details.cached_tokens (opencode #394)');

  const turn2 = JSON.parse(await chat(port, {
    model: 'muse-spark-1.3',
    messages: [
      { role: 'user', content: 'alpha one' },
      { role: 'assistant', content: 'RE:alpha one' },
      { role: 'user', content: 'beta two' }
    ]
  }));
  assert.equal(turn2.choices[0].message.content, 'RE:beta two');

  const log = fs.readFileSync(logFile, 'utf-8').trim().split('\n');
  assert.equal(log.filter(l => l.startsWith('START')).length, 1, 'one MSP session for the whole conversation');
  const turns = log.filter(l => l.startsWith('TURN'));
  assert.equal(turns.length, 2);
  assert.equal(turns[0], 'TURN alpha one');
  assert.equal(turns[1], 'TURN beta two', 'turn 2 must send only the new message, not the re-flattened history');
});

test('warm serve path: streaming turn streams the MSP deltas (#480)', async (t) => {
  const { port, stop } = await startProxy({ MUSE_PROXY_WARM: '1' });
  t.after(stop);
  const { content, finish } = streamedContent(await chat(port, {
    messages: [{ role: 'user', content: 'alpha one' }],
    stream: true
  }));
  assert.equal(content, 'RE:alpha one');
  assert.equal(finish, 'stop');
});

test('warm serve failure falls back to the exec path (#480)', async (t) => {
  const { port, stop } = await startProxy({ MUSE_PROXY_WARM: '1', STUB_SERVE_FAIL: '1' });
  t.after(stop);
  const plain = JSON.parse(await chat(port, { messages: [{ role: 'user', content: 'hi' }] }));
  assert.equal(plain.choices[0].message.content, 'Hello world', 'exec stub answer proves the fallback ran');
});

// --- CLI flag capability probe -------------------------------------------
// The fleet runs muse CLIs with different flag surfaces side by side
// (1.4.2 advertises neither serve --disable-request-user-input nor exec
// --disable-reminders; 1.4.13 advertises both). The proxy probes the
// resolved binary's --help once at startup and must pass only the flags
// it advertises — an unknown option kills the child outright.

const argvLines = (logFile, sub) => fs.readFileSync(logFile, 'utf-8')
  .split('\n')
  .filter(l => l.startsWith(`ARGV ${sub} `) && !l.includes('--help'));

const waitForProbe = async (port) => {
  const deadline = Date.now() + 5000;
  for (;;) {
    const res = await fetch(`http://127.0.0.1:${port}/health`);
    const health = await res.json();
    if (health.cliFlags && health.cliFlags.serveProbed && health.cliFlags.execProbed) return health;
    if (Date.now() > deadline) throw new Error('CLI flag probe did not complete');
    await new Promise(r => setTimeout(r, 25));
  }
};

test('flag probe: 1.4.2-shaped help — exec omits --disable-reminders, health reports the drops', async (t) => {
  const { port, stop, logFile } = await startProxy();
  t.after(stop);
  const health = await waitForProbe(port);
  assert.deepEqual(health.cliFlags.dropped.exec, ['--disable-reminders']);
  assert.deepEqual(health.cliFlags.dropped.serve, ['--disable-request-user-input']);

  const plain = JSON.parse(await chat(port, { messages: [{ role: 'user', content: 'hi' }] }));
  assert.equal(plain.choices[0].message.content, 'Hello world');
  const execArgv = argvLines(logFile, 'exec');
  assert.equal(execArgv.length, 1);
  assert.ok(execArgv[0].includes('--json'), 'core flags are still passed');
  assert.ok(execArgv[0].includes('--approval-mode never'), 'core flags are still passed');
  assert.ok(!execArgv[0].includes('--disable-reminders'), 'unadvertised flag must be omitted');
});

test('flag probe: 1.4.13-shaped help — exec passes --disable-reminders', async (t) => {
  const { port, stop, logFile } = await startProxy({ STUB_HELP_ALL_FLAGS: '1' });
  t.after(stop);
  const health = await waitForProbe(port);
  assert.deepEqual(health.cliFlags.dropped.exec, []);
  assert.deepEqual(health.cliFlags.dropped.serve, []);

  const plain = JSON.parse(await chat(port, { messages: [{ role: 'user', content: 'hi' }] }));
  assert.equal(plain.choices[0].message.content, 'Hello world');
  const execArgv = argvLines(logFile, 'exec');
  assert.equal(execArgv.length, 1);
  assert.ok(execArgv[0].includes('--disable-reminders'), 'advertised flag must be passed');
});

test('flag probe: 1.4.2-shaped help — warm serve spawn omits --disable-request-user-input', async (t) => {
  const { port, stop, logFile } = await startProxy({ MUSE_PROXY_WARM: '1' });
  t.after(stop);
  const plain = JSON.parse(await chat(port, {
    model: 'muse-spark-1.3',
    messages: [{ role: 'user', content: 'alpha one' }]
  }));
  assert.equal(plain.choices[0].message.content, 'RE:alpha one', 'warm path still answers');
  const serveArgv = argvLines(logFile, 'serve');
  assert.equal(serveArgv.length, 1);
  assert.ok(serveArgv[0].includes('--provider meta'), 'core flags are still passed');
  assert.ok(!serveArgv[0].includes('--disable-request-user-input'), 'unadvertised flag must be omitted');
});

test('flag probe: 1.4.13-shaped help — warm serve spawn passes --disable-request-user-input', async (t) => {
  const { port, stop, logFile } = await startProxy({ MUSE_PROXY_WARM: '1', STUB_HELP_ALL_FLAGS: '1' });
  t.after(stop);
  const plain = JSON.parse(await chat(port, {
    model: 'muse-spark-1.3',
    messages: [{ role: 'user', content: 'alpha one' }]
  }));
  assert.equal(plain.choices[0].message.content, 'RE:alpha one');
  const serveArgv = argvLines(logFile, 'serve');
  assert.equal(serveArgv.length, 1);
  assert.ok(serveArgv[0].includes('--disable-request-user-input'), 'advertised flag must be passed');
});

// --- Direct mode (#2753) ----------------------------------------------------
// The proxy exchanges the Muse CLI account token (`dca:...` in the temp
// HOME's .config/muse/auth.json) at a stub Meta API's /muse-code/key for a
// minted key, then speaks the Responses API upstream. The stub records
// every request so the tests can assert the exchange shape, the minted-key
// Authorization, the translation, and that Meta answering (even with 429)
// never falls back to a `muse exec` burn.

const RESPONSES_FIXTURE = {
  id: 'resp_stub',
  object: 'response',
  status: 'completed',
  model: 'muse-spark-1.3',
  output: [
    { type: 'message', id: 'msg_1', role: 'assistant', content: [{ type: 'output_text', text: 'Hello direct' }] },
    { type: 'function_call', id: 'fc_1', call_id: 'call_123', name: 'get_weather', arguments: '{"city":"Osseo"}', status: 'completed' }
  ],
  usage: { input_tokens: 13, output_tokens: 9, total_tokens: 22, input_tokens_details: { cached_tokens: 4 } }
};

const RESPONSES_SSE_FIXTURE = [
  ['response.output_text.delta', { type: 'response.output_text.delta', delta: 'Hello ' }],
  ['response.output_text.delta', { type: 'response.output_text.delta', delta: 'stream' }],
  ['response.output_item.added', { type: 'response.output_item.added', item: { type: 'function_call', id: 'fc_1', call_id: 'call_9', name: 'get_weather', arguments: '' } }],
  ['response.function_call_arguments.delta', { type: 'response.function_call_arguments.delta', item_id: 'fc_1', delta: '{"city":' }],
  ['response.function_call_arguments.delta', { type: 'response.function_call_arguments.delta', item_id: 'fc_1', delta: '"Osseo"}' }],
  ['response.completed', {
    type: 'response.completed',
    response: {
      id: 'resp_stream',
      status: 'completed',
      output: [
        { type: 'message', content: [{ type: 'output_text', text: 'Hello stream' }] },
        { type: 'function_call', call_id: 'call_9', name: 'get_weather', arguments: '{"city":"Osseo"}' }
      ],
      usage: { input_tokens: 11, output_tokens: 6, total_tokens: 17 }
    }
  }]
].map(([event, data]) => `event: ${event}\ndata: ${JSON.stringify(data)}\n\n`).join('');

const startMetaStub = async (mode) => {
  const requests = [];
  const server = http.createServer((req, res) => {
    let body = '';
    req.on('data', c => { body += c; });
    req.on('end', () => {
      requests.push({ method: req.method, url: req.url, headers: req.headers, body });
      if (req.url === '/muse-code/key') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          api_key: 'LLM|stub-minted',
          is_subs_active: true,
          require_payment: false,
          subs_tier_name: 'Muse Code High Usage'
        }));
        return;
      }
      if (req.url === '/v1/responses') {
        if (mode === 'attest') {
          res.writeHead(403, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({ error: { code: 'client_attestation_denied', message: 'This API key is billed to a Muse Code subscription. Muse Code subscriptions cover requests from the Muse Code client only.', type: 'permission_error' } }));
          return;
        }
        if (mode === '429') {
          res.writeHead(429, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({ error: { message: 'Subscription quota exhausted', type: 'rate_limit_error' } }));
          return;
        }
        if (mode === 'passthrough-tools') {
          let reqBody = {};
          try { reqBody = JSON.parse(body || '{}'); } catch { /* keep empty */ }
          const callItem = { type: 'function_call', call_id: 'call_pt1', name: 'terminal', arguments: '{"command":"echo hi"}' };
          if (reqBody.stream) {
            res.writeHead(200, { 'Content-Type': 'text/event-stream' });
            res.end(
              `data: ${JSON.stringify({ type: 'response.output_item.added', item: { ...callItem, status: 'in_progress' } })}\n\n` +
              `data: ${JSON.stringify({ type: 'response.completed', response: { id: 'resp_pt', status: 'completed', output: [callItem] } })}\n\n`);
          } else {
            res.writeHead(200, { 'Content-Type': 'application/json' });
            res.end(JSON.stringify({ id: 'resp_pt', object: 'response', status: 'completed', output: [callItem] }));
          }
          return;
        }
        if (mode === 'echo') {
          res.writeHead(200, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({ received: JSON.parse(body || '{}'), authorization: req.headers.authorization }));
          return;
        }
        let parsed = {};
        try { parsed = JSON.parse(body || '{}'); } catch { /* keep empty */ }
        if (parsed.stream) {
          res.writeHead(200, { 'Content-Type': 'text/event-stream' });
          res.end(mode === 'shell' ? RESPONSES_SSE_FIXTURE.replaceAll('get_weather', 'run_command') : RESPONSES_SSE_FIXTURE);
          return;
        }
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(mode === 'shell'
          ? JSON.stringify(RESPONSES_FIXTURE).replaceAll('get_weather', 'run_command')
          : JSON.stringify(RESPONSES_FIXTURE));
        return;
      }
      res.writeHead(404, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ error: { message: 'stub: unknown route' } }));
    });
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const { port } = server.address();
  return { url: `http://127.0.0.1:${port}`, requests, close: () => server.close() };
};

const writeStubAuth = (home) => {
  const dir = path.join(home, '.config', 'muse');
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, 'auth.json'), JSON.stringify({
    schema_version: 1,
    providers: {
      meta: {
        access_token: 'dca:stub-account-token',
        mechanism: 'oauth',
        obtained_via: 'device_code'
      }
    }
  }));
};

const parseChatSse = (sse) => {
  const chunks = [];
  let done = false;
  for (const line of sse.split('\n')) {
    if (line === 'data: [DONE]') { done = true; continue; }
    if (line.startsWith('data: ')) chunks.push(JSON.parse(line.slice('data: '.length)));
  }
  return { chunks, done };
};

test('direct mode: one exchange mints the key, chat is translated to Responses and back (#2753)', async (t) => {
  const meta = await startMetaStub('chat');
  t.after(() => meta.close());
  const { port, stop, home } = await startProxy({ MUSE_PROXY_META_API_BASE: meta.url });
  t.after(stop);
  writeStubAuth(home);

  const body = {
    model: 'muse-spark-1.3',
    reasoning_effort: 'max',
    max_tokens: 64,
    messages: [
      { role: 'system', content: 'Be terse.' },
      { role: 'user', content: 'weather in Osseo?' },
      { role: 'assistant', content: '', tool_calls: [{ id: 'call_0', type: 'function', function: { name: 'get_weather', arguments: '{"city":"Osseo"}' } }] },
      { role: 'tool', tool_call_id: 'call_0', content: 'sunny' },
      { role: 'user', content: 'thanks, and tomorrow?' }
    ],
    tools: [{ type: 'function', function: { name: 'get_weather', description: 'Get weather', parameters: { type: 'object', properties: { city: { type: 'string' } } } } }],
    tool_choice: { type: 'function', function: { name: 'get_weather' } }
  };
  const plain = JSON.parse(await chat(port, body));
  assert.equal(plain.model, 'muse-spark-1.3-max', 'model is the fullName the exec/warm handlers compute');
  assert.equal(plain.choices[0].message.content, 'Hello direct');
  assert.deepEqual(plain.choices[0].message.tool_calls, [{
    id: 'call_123',
    type: 'function',
    function: { name: 'get_weather', arguments: '{"city":"Osseo"}' }
  }]);
  assert.equal(plain.choices[0].finish_reason, 'tool_calls');
  assert.equal(plain.usage.prompt_tokens, 13);
  assert.equal(plain.usage.completion_tokens, 9);
  assert.equal(plain.usage.total_tokens, 22);
  assert.equal(plain.usage.prompt_tokens_details.cached_tokens, 4);

  // A second call must reuse the cached minted key: still one exchange.
  const again = JSON.parse(await chat(port, { messages: [{ role: 'user', content: 'hi again' }] }));
  assert.equal(again.choices[0].message.content, 'Hello direct');

  const exchanges = meta.requests.filter(r => r.url === '/muse-code/key');
  assert.equal(exchanges.length, 1, 'two chat calls must share one cached exchange');
  assert.equal(exchanges[0].method, 'POST');
  assert.equal(exchanges[0].headers.authorization, 'Bearer dca:stub-account-token');
  assert.equal(exchanges[0].headers['x-api-version'], '1.0.0');
  assert.deepEqual(JSON.parse(exchanges[0].body), { onboard: false });

  const upstream = meta.requests.filter(r => r.url === '/v1/responses');
  assert.equal(upstream.length, 2);
  for (const r of upstream) {
    assert.equal(r.headers.authorization, 'Bearer LLM|stub-minted', 'upstream sees the minted key, never the account token');
  }
  const sent = JSON.parse(upstream[0].body);
  assert.equal(sent.model, 'muse-spark-1.3');
  assert.equal(sent.instructions, 'Be terse.');
  assert.equal(sent.store, false);
  assert.deepEqual(sent.reasoning, { effort: 'xhigh' }, 'max clamps to the Responses ceiling xhigh');
  assert.equal(sent.max_output_tokens, 64);
  const fnCall = sent.input.find(i => i.type === 'function_call');
  assert.equal(fnCall.call_id, 'call_0');
  assert.equal(fnCall.name, 'get_weather');
  const fnOut = sent.input.find(i => i.type === 'function_call_output');
  assert.equal(fnOut.call_id, 'call_0');
  assert.equal(fnOut.output, 'sunny');
  assert.deepEqual(sent.tools, [{
    type: 'function',
    name: 'get_weather',
    description: 'Get weather',
    parameters: { type: 'object', properties: { city: { type: 'string' } } }
  }]);
  assert.deepEqual(sent.tool_choice, { type: 'function', name: 'get_weather' });

  const healthRes = await fetch(`http://127.0.0.1:${port}/health`);
  const health = await healthRes.json();
  assert.equal(health.direct.enabled, true);
  assert.equal(health.direct.credentialFound, true);
  assert.equal(health.direct.mintedKeyCached, true);
  assert.equal(health.direct.subsTier, 'Muse Code High Usage');
  assert.equal(health.direct.subsActive, true);
});

test('direct mode: Responses SSE is translated into chat chunks with tool calls (#2753)', async (t) => {
  const meta = await startMetaStub('chat');
  t.after(() => meta.close());
  const { port, stop, home } = await startProxy({ MUSE_PROXY_META_API_BASE: meta.url });
  t.after(stop);
  writeStubAuth(home);

  const { chunks, done } = parseChatSse(await chat(port, {
    messages: [{ role: 'user', content: 'weather?' }],
    stream: true
  }));
  assert.ok(done, 'stream terminates with [DONE]');
  const content = chunks.map(c => c.choices[0].delta.content || '').join('');
  assert.equal(content, 'Hello stream');
  const toolHeads = chunks.flatMap(c => c.choices[0].delta.tool_calls || []).filter(tc => tc.id);
  assert.equal(toolHeads.length, 1);
  assert.equal(toolHeads[0].id, 'call_9');
  assert.equal(toolHeads[0].function.name, 'get_weather');
  const toolArgs = chunks.flatMap(c => c.choices[0].delta.tool_calls || [])
    .map(tc => (tc.function && tc.function.arguments) || '')
    .join('');
  assert.equal(toolArgs, '{"city":"Osseo"}');
  const final = chunks.find(c => c.choices[0].finish_reason);
  assert.equal(final.choices[0].finish_reason, 'tool_calls');
  assert.equal(final.usage.prompt_tokens, 11);
  assert.equal(final.usage.completion_tokens, 6);
  assert.equal(final.usage.total_tokens, 17);
});

test('direct mode: upstream 429 passes through and never falls back to exec (#2753)', async (t) => {
  const meta = await startMetaStub('429');
  t.after(() => meta.close());
  const { port, stop, home, logFile } = await startProxy({ MUSE_PROXY_META_API_BASE: meta.url });
  t.after(stop);
  writeStubAuth(home);

  const res = await fetch(`http://127.0.0.1:${port}/v1/chat/completions`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ messages: [{ role: 'user', content: 'hi' }] })
  });
  assert.equal(res.status, 429, 'the upstream status is returned as-is');
  const body = JSON.parse(await res.text());
  assert.match(body.error.message, /Subscription quota exhausted/);
  assert.equal(meta.requests.filter(r => r.url === '/v1/responses').length, 1, 'a quota wall is never retried');
  const stubLog = fs.existsSync(logFile) ? fs.readFileSync(logFile, 'utf-8') : '';
  assert.ok(!stubLog.includes('--prompt-file'), 'no muse exec invocation after Meta answered');
});

test('direct mode: the shell tool is aliased upstream and its calls translate back', async (t) => {
  const meta = await startMetaStub('shell');
  t.after(() => meta.close());
  const { port, stop, home } = await startProxy({ MUSE_PROXY_META_API_BASE: meta.url });
  t.after(stop);
  writeStubAuth(home);

  const plain = JSON.parse(await chat(port, {
    messages: [
      { role: 'user', content: 'run echo hi' },
      { role: 'assistant', content: null, tool_calls: [{ id: 'call_0', type: 'function', function: { name: 'shell', arguments: '{"command":"echo hi"}' } }] },
      { role: 'tool', tool_call_id: 'call_0', content: 'hi' },
      { role: 'user', content: 'what did it print?' }
    ],
    tools: [{ type: 'function', function: { name: 'shell', description: 'Run a shell command', parameters: { type: 'object', properties: { command: { type: 'string' } } } } }]
  }));
  const upstream = JSON.parse(meta.requests.filter(r => r.url === '/v1/responses')[0].body);
  assert.deepEqual(upstream.tools.map(x => x.name), ['run_command'], 'the shell tool is presented upstream under its alias');
  const historyCall = upstream.input.find(i => i.type === 'function_call');
  assert.equal(historyCall.name, 'run_command', 'history tool calls are aliased too');
  assert.equal(plain.choices[0].message.tool_calls[0].function.name, 'shell', 'the model call is translated back to the client name');
});

test('direct mode: an attestation denial falls back to the CLI door', async (t) => {
  const meta = await startMetaStub('attest');
  t.after(() => meta.close());
  const { port, stop, home, logFile } = await startProxy({ MUSE_PROXY_META_API_BASE: meta.url });
  t.after(stop);
  writeStubAuth(home);

  const plain = JSON.parse(await chat(port, { messages: [{ role: 'user', content: 'hi' }] }));
  assert.equal(plain.choices[0].message.content, 'Hello world', 'exec stub answer proves the CLI fallback ran');
  const stubLog = fs.existsSync(logFile) ? fs.readFileSync(logFile, 'utf-8') : '';
  assert.ok(stubLog.includes('--prompt-file'), 'the CLI door took over after the denial');
});

test('direct mode: /v1/responses passes the body through verbatim, swapping only Authorization (#2753)', async (t) => {
  const meta = await startMetaStub('echo');
  t.after(() => meta.close());
  const { port, stop, home } = await startProxy({ MUSE_PROXY_META_API_BASE: meta.url });
  t.after(stop);
  writeStubAuth(home);

  const clientBody = {
    model: 'muse-wonder-1.0',
    input: [{ role: 'user', content: 'hi' }],
    stream: false,
    custom_field: { a: 1 }
  };
  const res = await fetch(`http://127.0.0.1:${port}/v1/responses`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', Authorization: 'Bearer client-token-must-be-dropped' },
    body: JSON.stringify(clientBody)
  });
  assert.equal(res.status, 200);
  const echoed = JSON.parse(await res.text());
  assert.deepEqual(echoed.received, clientBody, 'the upstream body is the client body verbatim');
  assert.equal(echoed.authorization, 'Bearer LLM|stub-minted');
  const upstream = meta.requests.filter(r => r.url === '/v1/responses');
  assert.equal(upstream.length, 1);
  assert.equal(upstream[0].body, JSON.stringify(clientBody), 'byte-for-byte passthrough on the wire');
});

test('direct mode: MUSE_PROXY_DIRECT=0 leaves the exec path in charge (#2753)', async (t) => {
  const meta = await startMetaStub('chat');
  t.after(() => meta.close());
  const { port, stop, home } = await startProxy({
    MUSE_PROXY_META_API_BASE: meta.url,
    MUSE_PROXY_DIRECT: '0'
  });
  t.after(stop);
  writeStubAuth(home);

  const plain = JSON.parse(await chat(port, { messages: [{ role: 'user', content: 'hi' }] }));
  assert.equal(plain.choices[0].message.content, 'Hello world', 'exec stub answer proves the CLI path ran');
  assert.equal(meta.requests.length, 0, 'the Meta API stub is never contacted');
});

test('direct mode: missing auth.json leaves the exec path in charge (#2753)', async (t) => {
  const meta = await startMetaStub('chat');
  t.after(() => meta.close());
  const { port, stop } = await startProxy({ MUSE_PROXY_META_API_BASE: meta.url });
  t.after(stop);

  const plain = JSON.parse(await chat(port, { messages: [{ role: 'user', content: 'hi' }] }));
  assert.equal(plain.choices[0].message.content, 'Hello world', 'exec stub answer proves the CLI path ran');
  assert.equal(meta.requests.length, 0, 'no credential means no exchange attempt');

  const healthRes = await fetch(`http://127.0.0.1:${port}/health`);
  const health = await healthRes.json();
  assert.equal(health.direct.enabled, true);
  assert.equal(health.direct.credentialFound, false);
  assert.equal(health.direct.mintedKeyCached, false);
});

test('direct mode: contributor ids reach upstream verbatim, effort suffix parses after contributor (#2753)', async (t) => {
  const meta = await startMetaStub('chat');
  t.after(() => meta.close());
  const { port, stop, home } = await startProxy({ MUSE_PROXY_META_API_BASE: meta.url });
  t.after(stop);
  writeStubAuth(home);

  const wonder = JSON.parse(await chat(port, {
    model: 'muse-wonder-1.0-contributor-high',
    messages: [{ role: 'user', content: 'hi' }]
  }));
  assert.equal(wonder.model, 'muse-wonder-1.0-contributor-high');

  const spark = JSON.parse(await chat(port, {
    model: 'muse-spark-1.3-contributor',
    messages: [{ role: 'user', content: 'hi' }]
  }));
  assert.equal(spark.model, 'muse-spark-1.3-contributor-high', 'no explicit effort means the default effort, contributor intact');

  const plainBase = JSON.parse(await chat(port, {
    model: 'muse-spark-1.3',
    messages: [{ role: 'user', content: 'hi' }]
  }));
  assert.equal(plainBase.model, 'muse-spark-1.3-high', 'a plain request is NOT rewritten to contributor');

  const sent = meta.requests.filter(r => r.url === '/v1/responses').map(r => JSON.parse(r.body));
  assert.equal(sent.length, 3);
  assert.equal(sent[0].model, 'muse-wonder-1.0-contributor', 'contributor id goes upstream verbatim');
  assert.deepEqual(sent[0].reasoning, { effort: 'high' }, 'the effort suffix after -contributor is the effort');
  assert.equal(sent[1].model, 'muse-spark-1.3-contributor');
  assert.deepEqual(sent[1].reasoning, { effort: 'high' });
  assert.equal(sent[2].model, 'muse-spark-1.3', 'plain base model stays plain upstream');
});

test('direct mode: /v1/responses passthrough aliases shell tools out and restores call names back (#2764)', async (t) => {
  const meta = await startMetaStub('passthrough-tools');
  t.after(() => meta.close());
  const { port, stop, home } = await startProxy({ MUSE_PROXY_META_API_BASE: meta.url });
  t.after(stop);
  writeStubAuth(home);

  const clientBody = {
    model: 'muse-spark-1.3-contributor',
    input: [
      { role: 'user', content: 'run echo hi' },
      { type: 'function_call', call_id: 'call_prev', name: 'bash', arguments: '{"command":"ls"}' }
    ],
    tools: [{ type: 'function', name: 'bash', description: 'Run a bash command', parameters: { type: 'object', properties: { command: { type: 'string' } } } }],
    tool_choice: { type: 'function', name: 'bash' },
    stream: false
  };
  const res = await fetch(`http://127.0.0.1:${port}/v1/responses`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', Authorization: 'Bearer client-side' },
    body: JSON.stringify(clientBody)
  });
  assert.equal(res.status, 200);
  const upstream = JSON.parse(meta.requests.filter(r => r.url === '/v1/responses')[0].body);
  assert.equal(upstream.tools[0].name, 'terminal', 'tool definitions are aliased upstream');
  assert.equal(upstream.input[1].name, 'terminal', 'history function_call items are aliased upstream');
  assert.equal(upstream.tool_choice.name, 'terminal', 'tool_choice is aliased upstream');
  assert.equal(upstream.authorization, undefined, 'no credential material in the body');
  const out = JSON.parse(await res.text());
  assert.equal(out.output[0].name, 'bash', 'the model call name is restored for the client');

  const sres = await fetch(`http://127.0.0.1:${port}/v1/responses`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', Authorization: 'Bearer client-side' },
    body: JSON.stringify({ ...clientBody, stream: true })
  });
  assert.equal(sres.status, 200);
  const streamText = await sres.text();
  assert.match(streamText, /"name":"bash"/, 'SSE events carry the restored client name');
  assert.ok(!streamText.includes('"name":"terminal"'), 'the alias never leaks to the client');
});
