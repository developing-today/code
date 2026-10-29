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
  return { port, stop: () => child.kill('SIGKILL'), logFile };
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
