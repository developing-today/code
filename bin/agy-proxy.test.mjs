// Tests for agy-proxy's failure detection (#678).
// Run: node --test bin/agy-proxy.test.mjs
//
// The proxy spawns one `agy` child per request, with the prompt delivered
// as the `-p` argument. These tests put a stub `agy` at
// $HOME/.local/bin/agy (the proxy's binary resolution prefers it) whose
// behaviour is keyed on markers in the prompt: a run that exited non-zero
// used to be reported as a successful 'stop' — in stream mode always
// (the close handler never checked the code), and in non-stream mode
// whenever the run had produced partial output.
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
prompt=""
fmt="json"
conv=""
prev=""
for a in "$@"; do
  if [ "$prev" = "-p" ]; then prompt="$a"; fi
  if [ "$prev" = "--conversation" ]; then conv="$a"; fi
  if [ "$a" = "stream-json" ]; then fmt="stream"; fi
  prev="$a"
done
case "$prompt" in
  *RETRYME*)
    if [ -n "$conv" ]; then
      echo "stub agy: resume failed" >&2
      exit 3
    fi ;;
  *WARMFAIL*)
    if [ -n "$conv" ]; then
      echo "stub agy: warm resume failed" >&2
      exit 3
    fi ;;
  *FAILME*)
    echo "stub agy: auth expired" >&2
    exit 3 ;;
  *FAILPARTIAL*)
    if [ "$fmt" = "stream" ]; then
      printf '%s\\n' '{"event":"step_update","step_update":{"text_delta":"partial answer"}}'
    else
      printf '%s\\n' '{"response":"partial answer"}'
    fi
    echo "stub agy: run failed midway" >&2
    exit 3 ;;
  *SIGKILLME*)
    kill -9 $$ ;;
esac
if [ "$fmt" = "stream" ]; then
  if [ -n "$conv" ]; then
    printf '{"event":"step_update","step_update":{"text_delta":"resumed: %s"}}\\n' "$prompt"
  else
    printf '%s\\n' '{"event":"step_update","step_update":{"text_delta":"Hello world"}}'
  fi
  printf '%s\\n' '{"event":"result","result":{"conversation_id":"stub-conv-42","usage":{"input_tokens":100,"output_tokens":3,"total_tokens":103}}}'
else
  if [ -n "$conv" ]; then
    printf '{"response":"resumed: %s","conversation_id":"%s","usage":{"input_tokens":100,"output_tokens":3,"total_tokens":103}}\\n' "$prompt" "$conv"
  else
    printf '%s\\n' '{"response":"Hello world","conversation_id":"stub-conv-42","usage":{"input_tokens":100,"output_tokens":3,"total_tokens":103}}'
  fi
fi
exit 0
`;

const freePort = () => new Promise((resolve, reject) => {
  const srv = net.createServer();
  srv.once('error', reject);
  srv.listen(0, '127.0.0.1', () => {
    const { port } = srv.address();
    srv.close(() => resolve(port));
  });
});

const startProxy = async (sharedTmp) => {
  const tmp = sharedTmp || fs.mkdtempSync(path.join(os.tmpdir(), 'agy-proxy-test-'));
  const home = path.join(tmp, 'home');
  fs.mkdirSync(path.join(home, '.local', 'bin'), { recursive: true });
  fs.writeFileSync(path.join(home, '.local', 'bin', 'agy'), STUB, { mode: 0o755 });
  const port = await freePort();
  const child = spawn(process.execPath, [path.join(binDir, 'agy-proxy.mjs')], {
    env: {
      ...process.env,
      HOME: home,
      XDG_STATE_HOME: path.join(tmp, 'state'),
      AGY_PROXY_PORT: String(port)
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

const post = (port, body, headers = {}) => fetch(`http://127.0.0.1:${port}/v1/chat/completions`, {
  method: 'POST',
  headers: { 'Content-Type': 'application/json', ...headers },
  body: JSON.stringify(body)
});

const parseSse = (sse) => {
  const dataLines = sse.split('\n').filter(l => l.startsWith('data: '));
  assert.equal(dataLines[dataLines.length - 1], 'data: [DONE]', 'stream must end with [DONE]');
  return dataLines.slice(0, -1).map(l => JSON.parse(l.slice('data: '.length)));
};

const assertStreamFailure = async (port, marker, detail) => {
  const messages = [{ role: 'user', content: `please ${marker} now` }];
  const streamRes = await post(port, { messages, stream: true });
  assert.equal(streamRes.status, 200);
  assert.match(streamRes.headers.get('content-type'), /text\/event-stream/);
  const events = parseSse(await streamRes.text());
  const errorEvent = events.find(e => e.error);
  assert.ok(errorEvent, `${marker}: stream must carry an error event`);
  assert.match(errorEvent.error.message, detail, `${marker}: stream detail must name the cause, got: ${errorEvent.error.message}`);
  const stopChunk = events.find(e => e.choices && e.choices[0].finish_reason === 'stop');
  assert.equal(stopChunk, undefined, `${marker}: no chunk may claim finish_reason 'stop'`);
};

test('successful run is unchanged in both modes', async (t) => {
  const { port, stop } = await startProxy();
  t.after(stop);
  const messages = [{ role: 'user', content: 'hi' }];

  const plainRes = await post(port, { messages });
  assert.equal(plainRes.status, 200);
  const plain = JSON.parse(await plainRes.text());
  assert.equal(plain.choices[0].message.content, 'Hello world');
  assert.equal(plain.choices[0].finish_reason, 'stop');

  const streamRes = await post(port, { messages, stream: true });
  assert.equal(streamRes.status, 200);
  const sse = await streamRes.text();
  assert.match(sse, /Hello world/);
  assert.match(sse, /"finish_reason":"stop"/);
  assert.ok(sse.trimEnd().endsWith('data: [DONE]'));
});

test('execution failure with no output: 502 non-stream, SSE error + [DONE] stream (#678)', async (t) => {
  const { port, stop } = await startProxy();
  t.after(stop);
  const messages = [{ role: 'user', content: 'please FAILME now' }];

  const plainRes = await post(port, { messages });
  assert.equal(plainRes.status, 502, 'non-stream failure must not be a 200 stop');
  const plainBody = JSON.parse(await plainRes.text());
  assert.match(plainBody.stderr, /stub agy: auth expired/);

  await assertStreamFailure(port, 'FAILME', /agy execution failed: stub agy: auth expired/);
});

test('partial output then failure: masked success becomes an error in both modes (#678)', async (t) => {
  const { port, stop } = await startProxy();
  t.after(stop);
  const messages = [{ role: 'user', content: 'please FAILPARTIAL now' }];

  const plainRes = await post(port, { messages });
  assert.equal(plainRes.status, 502, 'non-stream partial failure must not be a 200 stop');
  const plainBody = JSON.parse(await plainRes.text());
  assert.match(plainBody.stderr, /stub agy: run failed midway/);

  await assertStreamFailure(port, 'FAILPARTIAL', /agy execution failed: stub agy: run failed midway/);
});

test('signal-killed run: stream fallback names the signal, non-stream is a 502 (#678)', async (t) => {
  const { port, stop } = await startProxy();
  t.after(stop);
  const messages = [{ role: 'user', content: 'please SIGKILLME now' }];

  const plainRes = await post(port, { messages });
  assert.equal(plainRes.status, 502);
  const plainBody = JSON.parse(await plainRes.text());
  assert.equal(plainBody.signal, 'SIGKILL');

  await assertStreamFailure(port, 'SIGKILLME', /agy execution failed: agy process terminated by signal SIGKILL/);
});

test('warm session: continuation resumes with only the new turn (#480)', async (t) => {
  const { port, stop } = await startProxy();
  t.after(stop);

  const turn1 = [{ role: 'user', content: 'warm alpha' }];
  const r1 = await post(port, { messages: turn1 });
  assert.equal(r1.status, 200);
  const p1 = JSON.parse(await r1.text());
  assert.equal(p1.choices[0].message.content, 'Hello world');
  assert.equal(p1.conversation_id, 'stub-conv-42', 'conversation id is learned from the agy JSON output');

  const health = await (await fetch(`http://127.0.0.1:${port}/health`)).json();
  assert.equal(health.warmSessions, 1);

  const turn2 = [...turn1,
    { role: 'assistant', content: 'Hello world' },
    { role: 'user', content: 'warm beta' }];
  const r2 = await post(port, { messages: turn2 });
  assert.equal(r2.status, 200);
  const p2 = JSON.parse(await r2.text());
  assert.equal(p2.choices[0].message.content, 'resumed: warm beta',
    'resumed run receives ONLY the new user turn, not the flattened history');

  const turn3 = [...turn2,
    { role: 'assistant', content: p2.choices[0].message.content },
    { role: 'user', content: 'warm gamma' }];
  const r3 = await post(port, { messages: turn3, stream: true });
  assert.equal(r3.status, 200);
  const sse = await r3.text();
  assert.match(sse, /resumed: warm gamma/);
  assert.match(sse, /"finish_reason":"stop"/);
});

test('warm session: tampered history or repeated turn falls back to a fresh full run (#480)', async (t) => {
  const { port, stop } = await startProxy();
  t.after(stop);

  const turn1 = [{ role: 'user', content: 'warm alpha' }];
  const r1 = await post(port, { messages: turn1 });
  assert.equal(r1.status, 200);
  await r1.text();

  // Same single-turn request again: message count <= fedCount, so the
  // stored conversation must NOT be resumed.
  const repeat = await post(port, { messages: turn1 });
  assert.equal(repeat.status, 200);
  assert.equal(JSON.parse(await repeat.text()).choices[0].message.content, 'Hello world');

  // Assistant echo that is not the response agy produced: mismatch.
  const tampered = [...turn1,
    { role: 'assistant', content: 'something else entirely' },
    { role: 'user', content: 'warm beta' }];
  const r2 = await post(port, { messages: tampered });
  assert.equal(r2.status, 200);
  assert.equal(JSON.parse(await r2.text()).choices[0].message.content, 'Hello world',
    'mismatched history must be a fresh full run, not a resume');
});

test('warm session: failed warm resume still answers via a fresh full run (#480)', async (t) => {
  const { port, stop } = await startProxy();
  t.after(stop);

  const turn1 = [{ role: 'user', content: 'warm alpha' }];
  const r1 = await post(port, { messages: turn1 });
  assert.equal(r1.status, 200);
  await r1.text();

  const turn2 = [...turn1,
    { role: 'assistant', content: 'Hello world' },
    { role: 'user', content: 'please WARMFAIL now' }];
  const r2 = await post(port, { messages: turn2 });
  assert.equal(r2.status, 200, 'a failed resume must never fail the request');
  assert.equal(JSON.parse(await r2.text()).choices[0].message.content, 'Hello world');
});

test('warm session: store persists across a proxy restart (#480)', async (t) => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'agy-proxy-test-'));
  const first = await startProxy(tmp);
  const turn1 = [{ role: 'user', content: 'warm alpha' }];
  const r1 = await post(first.port, { messages: turn1 });
  assert.equal(r1.status, 200);
  await r1.text();
  first.stop();
  // Give the killed process a moment to release the port/state dir.
  await new Promise(r => setTimeout(r, 200));
  assert.ok(
    fs.existsSync(path.join(tmp, 'state', 'agy-proxy', 'warm-sessions.json')),
    'warm sessions are persisted next to endpoint.json');

  const second = await startProxy(tmp);
  t.after(second.stop);
  const turn2 = [...turn1,
    { role: 'assistant', content: 'Hello world' },
    { role: 'user', content: 'warm beta' }];
  const r2 = await post(second.port, { messages: turn2 });
  assert.equal(r2.status, 200);
  assert.equal(JSON.parse(await r2.text()).choices[0].message.content, 'resumed: warm beta',
    'a restarted proxy still resumes the persisted conversation');
});

test('failed resume still retries once as a fresh conversation (#678)', async (t) => {
  const { port, stop } = await startProxy();
  t.after(stop);
  const messages = [{ role: 'user', content: 'please RETRYME now' }];
  const headers = { 'x-conversation-id': 'conv-123' };

  const plainRes = await post(port, { messages }, headers);
  assert.equal(plainRes.status, 200);
  const plain = JSON.parse(await plainRes.text());
  assert.equal(plain.choices[0].message.content, 'Hello world');
  assert.equal(plain.choices[0].finish_reason, 'stop');

  const streamRes = await post(port, { messages, stream: true }, headers);
  assert.equal(streamRes.status, 200);
  const sse = await streamRes.text();
  assert.match(sse, /Hello world/);
  assert.match(sse, /"finish_reason":"stop"/);
});
