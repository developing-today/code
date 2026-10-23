// Tests for codex-proxy's failure detection (#676).
// Run: node --test bin/codex-proxy.test.mjs
//
// The proxy spawns one `codex exec` child per request, with the prompt
// delivered on stdin. These tests put a stub `codex` at
// $HOME/.local/bin/codex (the proxy's binary resolution prefers it)
// whose behaviour is keyed on markers in the prompt: a run that exits
// non-zero used to be reported as a successful 'stop' — empty when it
// produced nothing, and carrying the truncated text when it produced
// partial output.
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
prompt="$(cat)"
case "$prompt" in
  *FAILME*)
    echo "stub codex: auth expired" >&2
    exit 3 ;;
  *FAILPARTIAL*)
    printf '%s\\n' '{"type":"item.completed","item":{"type":"agent_message","text":"partial answer"}}'
    echo "stub codex: run failed midway" >&2
    exit 3 ;;
  *SIGKILLME*)
    kill -9 $$ ;;
  *)
    printf '%s\\n' '{"type":"item.completed","item":{"type":"agent_message","text":"Hello world"}}'
    printf '%s\\n' '{"type":"turn.completed","usage":{"input_tokens":100,"output_tokens":3}}'
    exit 0 ;;
esac
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
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'codex-proxy-test-'));
  const home = path.join(tmp, 'home');
  fs.mkdirSync(path.join(home, '.local', 'bin'), { recursive: true });
  fs.writeFileSync(path.join(home, '.local', 'bin', 'codex'), STUB, { mode: 0o755 });
  const port = await freePort();
  const child = spawn(process.execPath, [path.join(binDir, 'codex-proxy.mjs')], {
    env: {
      ...process.env,
      HOME: home,
      XDG_STATE_HOME: path.join(tmp, 'state'),
      CODEX_PROXY_PORT: String(port)
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

const post = (port, body) => fetch(`http://127.0.0.1:${port}/v1/chat/completions`, {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify(body)
});

const assertFailure = async (port, marker, detail) => {
  const messages = [{ role: 'user', content: `please ${marker} now` }];

  const plainRes = await post(port, { messages });
  assert.equal(plainRes.status, 500, `${marker}: non-stream must be a 500, not a 200 'stop'`);
  const plainBody = JSON.parse(await plainRes.text());
  assert.match(plainBody.error.message, detail, `${marker}: non-stream detail must name the cause, got: ${plainBody.error.message}`);
  assert.doesNotMatch(plainBody.error.message, /code (null|-2)\b/);

  const streamRes = await post(port, { messages, stream: true });
  assert.equal(streamRes.status, 200);
  assert.match(streamRes.headers.get('content-type'), /text\/event-stream/);
  const sse = await streamRes.text();
  const dataLines = sse.split('\n').filter(l => l.startsWith('data: '));
  assert.equal(dataLines[dataLines.length - 1], 'data: [DONE]', `${marker}: stream must end with [DONE]`);
  const events = dataLines.slice(0, -1).map(l => JSON.parse(l.slice('data: '.length)));
  const errorEvent = events.find(e => e.error);
  assert.ok(errorEvent, `${marker}: stream must carry an error event, got: ${JSON.stringify(sse)}`);
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

test('execution failure with no output: 500 non-stream, SSE error + [DONE] stream (#676)', async (t) => {
  const { port, stop } = await startProxy();
  t.after(stop);
  await assertFailure(port, 'FAILME', /Codex execution failed: stub codex: auth expired/);
});

test('partial output then failure: masked success becomes an error in both modes (#676)', async (t) => {
  const { port, stop } = await startProxy();
  t.after(stop);
  await assertFailure(port, 'FAILPARTIAL', /Codex execution failed: stub codex: run failed midway/);
});

test('empty-stderr fallback names the killing signal, not code null (#676)', async (t) => {
  const { port, stop } = await startProxy();
  t.after(stop);
  await assertFailure(port, 'SIGKILLME', /Codex execution failed: codex exec terminated by signal SIGKILL/);
});
