// Tests for codex-proxy's failure detection (#676, #680).
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
resume=0
for a in "$@"; do [ "$a" = "resume" ] && resume=1; done
{
  echo "ARGS:$*"
  echo "RESUME:$resume"
  echo "PROMPT:$prompt"
  echo "==="
} >> "$HOME/invocations.log"
if [ "$resume" = "1" ]; then
  case "$prompt" in
    *RESUMEFAIL*)
      echo "stub codex: session not found" >&2
      exit 3 ;;
    *)
      printf '%s\\n' '{"type":"thread.started","thread_id":"stub-session-1"}'
      printf '%s\\n' '{"type":"item.completed","item":{"type":"agent_message","text":"resumed answer"}}'
      printf '%s\\n' '{"type":"turn.completed","usage":{"input_tokens":10,"output_tokens":3}}'
      exit 0 ;;
  esac
fi
case "$prompt" in
  *FAILME*)
    echo "stub codex: auth expired" >&2
    exit 3 ;;
  *FAILPARTIAL*)
    printf '%s\\n' '{"type":"item.completed","item":{"type":"agent_message","text":"partial answer"}}'
    echo "stub codex: run failed midway" >&2
    exit 3 ;;
  *INBANDPARTIAL*)
    printf '%s\\n' '{"type":"item.completed","item":{"type":"agent_message","text":"partial answer"}}'
    printf '%s\\n' '{"type":"turn.failed","error":{"message":"stub codex: run failed midway"}}'
    exit 0 ;;
  *INBANDFAIL*)
    printf '%s\\n' '{"type":"turn.failed","error":{"message":"stub codex: model overloaded"}}'
    exit 0 ;;
  *SIGKILLME*)
    kill -9 $$ ;;
  *)
    printf '%s\\n' '{"type":"thread.started","thread_id":"stub-session-1"}'
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

const startProxy = async (reuse) => {
  const tmp = reuse ? reuse.tmp : fs.mkdtempSync(path.join(os.tmpdir(), 'codex-proxy-test-'));
  const home = path.join(tmp, 'home');
  fs.mkdirSync(path.join(home, '.local', 'bin'), { recursive: true });
  fs.writeFileSync(path.join(home, '.local', 'bin', 'codex'), STUB, { mode: 0o755 });
  const stateHome = path.join(tmp, 'state');
  const port = await freePort();
  const child = spawn(process.execPath, [path.join(binDir, 'codex-proxy.mjs')], {
    env: {
      ...process.env,
      HOME: home,
      XDG_STATE_HOME: stateHome,
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
  return { port, tmp, home, stateHome, stop: () => child.kill('SIGKILL') };
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

test('turn.failed with exit 0 and no output: 500 non-stream, SSE error + [DONE] stream (#680)', async (t) => {
  const { port, stop } = await startProxy();
  t.after(stop);
  await assertFailure(port, 'INBANDFAIL', /Codex execution failed: stub codex: model overloaded/);
});

test('partial output then turn.failed with exit 0: masked success becomes an error in both modes (#680)', async (t) => {
  const { port, stop } = await startProxy();
  t.after(stop);
  await assertFailure(port, 'INBANDPARTIAL', /Codex execution failed: stub codex: run failed midway/);
});

test('empty-stderr fallback names the killing signal, not code null (#676)', async (t) => {
  const { port, stop } = await startProxy();
  t.after(stop);
  await assertFailure(port, 'SIGKILLME', /Codex execution failed: codex exec terminated by signal SIGKILL/);
});

const readInvocations = (home) => {
  const raw = fs.readFileSync(path.join(home, 'invocations.log'), 'utf-8');
  return raw
    .split('===\n')
    .filter(s => s.trim())
    .map(block => {
      const resume = /RESUME:(\d)/.exec(block);
      const prompt = /PROMPT:([\s\S]*)$/.exec(block);
      return { resume: resume && resume[1] === '1', prompt: prompt ? prompt[1] : '' };
    });
};

const TURN1 = [{ role: 'user', content: 'alpha first question' }];
const TURN2 = [
  { role: 'user', content: 'alpha first question' },
  { role: 'assistant', content: 'Hello world' },
  { role: 'user', content: 'beta second question' }
];

test('second turn resumes the warm session with only the new tail (#480)', async (t) => {
  const { port, home, stateHome, stop } = await startProxy();
  t.after(stop);

  const r1 = await post(port, { messages: TURN1 });
  assert.equal(r1.status, 200);
  assert.equal(JSON.parse(await r1.text()).choices[0].message.content, 'Hello world');

  // The captured session id is persisted next to endpoint.json.
  const saved = JSON.parse(
    fs.readFileSync(path.join(stateHome, 'codex-proxy', 'sessions.json'), 'utf-8')
  );
  const entries = Object.values(saved);
  assert.equal(entries.length, 1);
  assert.equal(entries[0].sessionId, 'stub-session-1');
  assert.equal(entries[0].fedCount, 1);

  const r2 = await post(port, { messages: TURN2 });
  assert.equal(r2.status, 200);
  assert.equal(JSON.parse(await r2.text()).choices[0].message.content, 'resumed answer');

  const invocations = readInvocations(home);
  assert.equal(invocations.length, 2);
  assert.equal(invocations[0].resume, false);
  assert.equal(invocations[1].resume, true, 'second turn must use exec resume');
  assert.match(invocations[1].prompt, /beta second question/);
  assert.doesNotMatch(invocations[1].prompt, /alpha first question/);
});

test('resume failure falls back to a full-history run and still answers (#480)', async (t) => {
  const { port, home, stop } = await startProxy();
  t.after(stop);

  const r1 = await post(port, { messages: TURN1 });
  assert.equal(r1.status, 200);
  await r1.text();

  const turn2Fail = [
    { role: 'user', content: 'alpha first question' },
    { role: 'assistant', content: 'Hello world' },
    { role: 'user', content: 'please RESUMEFAIL now' }
  ];
  const r2 = await post(port, { messages: turn2Fail });
  assert.equal(r2.status, 200, 'a failed resume must not fail the request');
  assert.equal(JSON.parse(await r2.text()).choices[0].message.content, 'Hello world');

  const invocations = readInvocations(home);
  assert.equal(invocations.length, 3, 'expected fresh, resume, then full fallback');
  assert.equal(invocations[1].resume, true);
  assert.equal(invocations[2].resume, false);
  assert.match(invocations[2].prompt, /alpha first question/, 'fallback must feed the full history');
  assert.match(invocations[2].prompt, /RESUMEFAIL/);
});

test('changed system prompt does not resume; full run instead (#480)', async (t) => {
  const { port, home, stop } = await startProxy();
  t.after(stop);

  const t1 = [
    { role: 'system', content: 'be terse' },
    { role: 'user', content: 'alpha first question' }
  ];
  const r1 = await post(port, { messages: t1 });
  assert.equal(r1.status, 200);
  await r1.text();

  const t2 = [
    { role: 'system', content: 'be verbose' },
    { role: 'user', content: 'alpha first question' },
    { role: 'assistant', content: 'Hello world' },
    { role: 'user', content: 'beta second question' }
  ];
  const r2 = await post(port, { messages: t2 });
  assert.equal(r2.status, 200);
  assert.equal(JSON.parse(await r2.text()).choices[0].message.content, 'Hello world');

  const invocations = readInvocations(home);
  assert.equal(invocations.length, 2);
  assert.equal(invocations[1].resume, false, 'system change must bypass resume');
});

test('session map survives a proxy restart via sessions.json (#480)', async (t) => {
  const first = await startProxy();
  const r1 = await post(first.port, { messages: TURN1 });
  assert.equal(r1.status, 200);
  await r1.text();
  first.stop();

  const second = await startProxy(first);
  t.after(second.stop);
  const r2 = await post(second.port, { messages: TURN2 });
  assert.equal(r2.status, 200);
  assert.equal(JSON.parse(await r2.text()).choices[0].message.content, 'resumed answer');

  const invocations = readInvocations(first.home);
  assert.equal(invocations.length, 2);
  assert.equal(invocations[1].resume, true, 'restarted proxy must still resume');
});
