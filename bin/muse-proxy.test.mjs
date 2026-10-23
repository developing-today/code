// Tests for muse-proxy's streaming/terminal-text reconciliation (#484).
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

const STUB = `#!/bin/sh
# The prompt file path is the argument after --prompt-file.
prompt=""
prev=""
for a in "$@"; do
  if [ "$prev" = "--prompt-file" ]; then prompt="$a"; fi
  prev="$a"
done
if grep -q FAILME "$prompt" 2>/dev/null; then
  echo "stub muse: auth expired" >&2
  exit 3
fi
if grep -q FAILPARTIAL "$prompt" 2>/dev/null; then
  printf '%s\\n' '{"payload_type":"run.output.delta","payload":{"text":"partial answer"}}'
  printf '%s\\n' '{"payload_type":"run.terminal.failed","payload":{"terminal":"failed","text":"partial answer"}}'
  echo "stub muse: run failed midway" >&2
  exit 0
fi
if grep -q CRASHPARTIAL "$prompt" 2>/dev/null; then
  printf '%s\\n' '{"payload_type":"run.output.delta","payload":{"text":"partial answer"}}'
  echo "stub muse: crashed midway" >&2
  exit 3
fi
if grep -q DIVERGE "$prompt" 2>/dev/null; then
  printf '%s\\n' '{"payload_type":"run.output.delta","payload":{"text":"abc"}}'
  printf '%s\\n' '{"payload_type":"run.terminal.completed","payload":{"terminal":"completed","text":"xyz-longer"}}'
else
  printf '%s\\n' '{"payload_type":"run.output.delta","payload":{"text":"Hel"}}'
  printf '%s\\n' '{"payload_type":"run.usage","payload":{"models":[{"input_tokens":100,"output_tokens":3}]}}'
  printf '%s\\n' '{"payload_type":"run.terminal.completed","payload":{"terminal":"completed","text":"Hello world"}}'
fi
`;

const freePort = () => new Promise((resolve, reject) => {
  const srv = net.createServer();
  srv.once('error', reject);
  srv.listen(0, '127.0.0.1', () => {
    const { port } = srv.address();
    srv.close(() => resolve(port));
  });
});

const startProxy = async () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'muse-proxy-test-'));
  const stubDir = path.join(tmp, 'stub');
  fs.mkdirSync(stubDir);
  fs.mkdirSync(path.join(tmp, 'home'));
  fs.writeFileSync(path.join(stubDir, 'muse'), STUB, { mode: 0o755 });
  const port = await freePort();
  const child = spawn(process.execPath, [path.join(binDir, 'muse-proxy.mjs')], {
    env: {
      ...process.env,
      HOME: path.join(tmp, 'home'),
      PATH: `${stubDir}:${process.env.PATH}`,
      MUSE_PROXY_PORT: String(port),
      MUSE_PROXY_WORKDIR: path.join(tmp, 'work')
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
  return { port, stop: () => child.kill('SIGKILL') };
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
