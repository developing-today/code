// Tests for agy-proxy: failure detection (#678), the live-pulled model
// list (fleet live-pull rule, 2026-10-28; #1692's unknown-model 400 is
// judged against it), and the resident warm stream-json serving path.
// Run: node --test bin/agy-proxy.test.mjs
//
// The proxy now holds resident `agy` processes speaking
// `--input-format stream-json`: prompts arrive on the child's stdin as
// NDJSON `{"event":"user","message":{"content":...}}` lines and each
// turn ends with a `result` event. The served model list is pulled
// live from `agy models`. These tests put a stub `agy` at
// $HOME/.local/bin/agy (the proxy's binary resolution reaches it when
// no system agy exists) that speaks that protocol AND answers
// `agy models` with a TSV list including a test-only model no static
// array could contain (startProxy also pins the stub via
// AGY_PROXY_AGY_BIN so a host-installed agy can never shadow it,
// #3355): it emits an `init` event at spawn, then one
// reply per stdin line, keyed on markers in the prompt — a run that
// dies non-zero must be reported as an error, never as a successful
// 'stop' (in stream mode always, in non-stream mode also when the run
// produced partial output first).
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
if [ "$1" = "models" ]; then
  if [ -n "$AGY_STUB_FAIL_MODELS" ]; then
    echo "stub agy: models unavailable" >&2
    exit 1
  fi
  printf '%s\\t%s\\n' 'gemini-4-argon-medium' 'Gemini 4 Argon (Medium)'
  printf '%s\\t%s\\n' 'gemini-4-argon-high' 'Gemini 4 Argon (High)'
  printf '%s\\t%s\\n' 'gemini-4-argon-low' 'Gemini 4 Argon (Low)'
  printf '%s\\t%s\\n' 'gemini-3.8-flash-high' 'Gemini 3.8 Flash (High)'
  printf '%s\\t%s\\n' 'gemini-3.8-flash-medium' 'Gemini 3.8 Flash (Medium)'
  printf '%s\\t%s\\n' 'gemini-3.8-flash-low' 'Gemini 3.8 Flash (Low)'
  printf '%s\\t%s\\n' 'claude-opus-5-5-low' 'Claude Opus 5.5 (Low)'
  printf '%s\\t%s\\n' 'claude-opus-5-5-medium' 'Claude Opus 5.5 (Medium)'
  printf '%s\\t%s\\n' 'claude-opus-5-5-high' 'Claude Opus 5.5 (High)'
  printf '%s\\t%s\\n' 'claude-sonnet-5-5-low' 'Claude Sonnet 5.5 (Low)'
  printf '%s\\t%s\\n' 'claude-sonnet-5-5-medium' 'Claude Sonnet 5.5 (Medium)'
  printf '%s\\t%s\\n' 'claude-sonnet-5-5-high' 'Claude Sonnet 5.5 (High)'
  printf '%s\\t%s\\n' 'gpt-oss-120b-medium' 'GPT-OSS 120B (Medium)'
  printf '%s\\t%s\\n' 'gemini-9-testonly-medium' 'Gemini 9 Testonly (Medium)'
  exit 0
fi
conv=""
prev=""
for a in "$@"; do
  if [ "$prev" = "--conversation" ]; then conv="$a"; fi
  prev="$a"
done
printf '%s\\n' '{"event":"init","conversation_id":"stub-conv-1","init":{}}'
while IFS= read -r line; do
  case "$line" in
    *RETRYME*)
      if [ -n "$conv" ]; then
        echo "stub agy: resume failed" >&2
        exit 3
      fi ;;
    *FAILME*)
      echo "stub agy: auth expired" >&2
      exit 3 ;;
    *FAILPARTIAL*)
      printf '%s\\n' '{"event":"step_update","step_update":{"text_delta":"partial answer"}}'
      echo "stub agy: run failed midway" >&2
      exit 3 ;;
    *FAILTAIL*)
      if [ -n "$conv" ]; then
        printf '%s' '{"event":"step_update","step_update":{"text_delta":"partial tail"}}'
        echo "stub agy: tail run failed" >&2
        exit 3
      fi ;;
    *SIGKILLME*)
      kill -9 $$ ;;
  esac
  printf '%s\\n' '{"event":"step_update","step_update":{"text_delta":"Hello world"}}'
  printf '%s\\n' '{"event":"result","result":{"conversation_id":"stub-conv-1","status":"SUCCESS","response":"Hello world","usage":{"input_tokens":100,"output_tokens":3,"total_tokens":103}}}'
done
`;

const freePort = () => new Promise((resolve, reject) => {
  const srv = net.createServer();
  srv.once('error', reject);
  srv.listen(0, '127.0.0.1', () => {
    const { port } = srv.address();
    srv.close(() => resolve(port));
  });
});

const startProxy = async (extraEnv = {}) => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'agy-proxy-test-'));
  const home = path.join(tmp, 'home');
  fs.mkdirSync(path.join(home, '.local', 'bin'), { recursive: true });
  fs.writeFileSync(path.join(home, '.local', 'bin', 'agy'), STUB, { mode: 0o755 });
  const port = await freePort();
  const child = spawn(process.execPath, [path.join(binDir, 'agy-proxy.mjs')], {
    env: {
      ...process.env,
      HOME: home,
      XDG_STATE_HOME: path.join(tmp, 'state'),
      AGY_PROXY_PORT: String(port),
      AGY_PROXY_AGY_BIN: path.join(home, ".local", "bin", "agy"),
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

test('unterminated final delta counts as output: failed resume is NOT retried fresh (#683)', async (t) => {
  const { port, stop } = await startProxy();
  t.after(stop);
  const messages = [{ role: 'user', content: 'please FAILTAIL now' }];
  const headers = { 'x-conversation-id': 'conv-456' };

  const plainRes = await post(port, { messages }, headers);
  assert.equal(plainRes.status, 502, 'a resumed turn whose only output is an unterminated tail delta must fail, not silently retry as a fresh conversation');
  const plainBody = JSON.parse(await plainRes.text());
  assert.match(plainBody.stderr, /stub agy: tail run failed/);

  const streamRes = await post(port, { messages, stream: true }, headers);
  assert.equal(streamRes.status, 200);
  const events = parseSse(await streamRes.text());
  const deltaChunk = events.find(e => e.choices && e.choices[0].delta && e.choices[0].delta.content === 'partial tail');
  assert.ok(deltaChunk, 'the unterminated tail delta must be delivered exactly like a terminated line');
  const errorEvent = events.find(e => e.error);
  assert.ok(errorEvent, 'stream must carry an error event');
  assert.match(errorEvent.error.message, /tail run failed/);
  const stopChunk = events.find(e => e.choices && e.choices[0].finish_reason === 'stop');
  assert.equal(stopChunk, undefined, "no chunk may claim finish_reason 'stop'");
});

test('session requests reuse one resident worker (same pid across turns)', async (t) => {
  const { port, stop } = await startProxy();
  t.after(stop);
  const headers = { 'x-conversation-id': 'conv-warm-1' };

  const first = await post(port, { messages: [{ role: 'user', content: 'hi' }] }, headers);
  assert.equal(first.status, 200);
  await first.text();
  const pid1 = first.headers.get('x-agy-worker-pid');
  assert.ok(pid1 && Number(pid1) > 0, 'first response must name its worker pid');

  const second = await post(port, {
    messages: [
      { role: 'user', content: 'hi' },
      { role: 'assistant', content: 'Hello world' },
      { role: 'user', content: 'again' }
    ]
  }, headers);
  assert.equal(second.status, 200);
  const secondBody = JSON.parse(await second.text());
  assert.equal(secondBody.choices[0].message.content, 'Hello world');
  const pid2 = second.headers.get('x-agy-worker-pid');
  assert.equal(pid2, pid1, 'second turn of a session must run on the same resident worker');
  assert.ok(Number(second.headers.get('x-agy-turn-ms')) >= 0, 'responses must carry per-turn timing (x-agy-turn-ms)');

  const health = await (await fetch(`http://127.0.0.1:${port}/health`)).json();
  assert.equal(health.engine, 'agy-stream-json-resident');
  assert.ok(health.warm.sessionPids.includes(Number(pid1)), 'health must list the live session worker pid');
});

test('a pre-warmed spare is ready before the first stateless request', async (t) => {
  const { port, stop } = await startProxy();
  t.after(stop);
  const deadline = Date.now() + 5000;
  for (;;) {
    const health = await (await fetch(`http://127.0.0.1:${port}/health`)).json();
    if (Array.isArray(health.warm?.sparePids) && health.warm.sparePids.length > 0) {
      assert.ok(health.warm.spares['gemini-4-argon-medium'] > 0, 'argon spare must be pre-warmed');
      return;
    }
    if (Date.now() > deadline) throw new Error('no pre-warmed spare appeared in /health');
    await new Promise(r => setTimeout(r, 50));
  }
});

test("unknown model is a 400, never a silent fallback; the served list is the live `agy models` list", async (t) => {
  const { port, stop } = await startProxy();
  t.after(stop);
  const messages = [{ role: "user", content: "hi" }];

  const badRes = await post(port, { model: "gemini-9-nonexistent", messages });
  assert.equal(badRes.status, 400, "unknown model must fail loudly, not fall back to the first model");
  const badBody = JSON.parse(await badRes.text());
  assert.match(badBody.error, /gemini-9-nonexistent/);

  const listRes = await fetch(`http://127.0.0.1:${port}/v1/models`);
  const list = JSON.parse(await listRes.text());
  const ids = list.data.map((m) => m.id);
  // The stub's `agy models` answer lists these 13 real ids (matching
  // agy 1.2.16 on 2026-10-28); the proxy serves each base plus its
  // per-effort variants, sourced live from that answer.
  for (const id of [
    "gemini-4-argon", "gemini-4-argon-low", "gemini-4-argon-medium", "gemini-4-argon-high",
    "gemini-3.8-flash", "gemini-3.8-flash-low", "gemini-3.8-flash-medium", "gemini-3.8-flash-high",
    "claude-sonnet-5-5", "claude-sonnet-5-5-low", "claude-sonnet-5-5-medium", "claude-sonnet-5-5-high",
    "claude-opus-5-5", "claude-opus-5-5-low", "claude-opus-5-5-medium", "claude-opus-5-5-high",
    "gpt-oss-120b", "gpt-oss-120b-medium"
  ]) {
    assert.ok(ids.includes(id), `models list must include ${id}`);
  }
  // Advertised before 2026-10-28 but absent from `agy models` (agy
  // itself rejects them): they must not be served or silently remapped.
  for (const id of ["gemini-3.7-flash", "gemini-3.6-flash", "gemini-3.1-pro"]) {
    assert.ok(!ids.includes(id), `models list must not include ${id}`);
    const res = await post(port, { model: id, messages });
    assert.equal(res.status, 400, `${id} must be a 400, not a silent remap`);
    await res.text();
  }

  const argonRes = await post(port, { model: "gemini-4-argon", messages });
  assert.equal(argonRes.status, 200);
  const argon = JSON.parse(await argonRes.text());
  assert.equal(argon.model, "gemini-4-argon-medium");

  const sonnetRes = await post(port, { model: "claude-sonnet-5-5", messages });
  assert.equal(sonnetRes.status, 200);
  const sonnet = JSON.parse(await sonnetRes.text());
  assert.equal(sonnet.model, "claude-sonnet-5-5-medium");
  assert.equal(sonnet.choices[0].message.content, "Hello world");

  const opusRes = await post(port, { model: "claude-opus-5-5-high", messages });
  assert.equal(opusRes.status, 200);
  const opus = JSON.parse(await opusRes.text());
  assert.equal(opus.model, "claude-opus-5-5-high");
});

test('model list is sourced live: a model only the stub`s `agy models` reports is served', async (t) => {
  const { port, stop } = await startProxy();
  t.after(stop);
  const messages = [{ role: 'user', content: 'hi' }];

  const deadline = Date.now() + 5000;
  let health;
  for (;;) {
    health = await (await fetch(`http://127.0.0.1:${port}/health`)).json();
    if (health.models.source === 'live') break;
    if (Date.now() > deadline) break;
    await new Promise(r => setTimeout(r, 50));
  }
  assert.equal(health.models.source, 'live', 'catalog must come from the live pull');

  const listRes = await fetch(`http://127.0.0.1:${port}/v1/models`);
  const list = JSON.parse(await listRes.text());
  const ids = list.data.map((m) => m.id);
  assert.ok(ids.includes('gemini-9-testonly'), 'live list must include the stub-only base');
  assert.ok(ids.includes('gemini-9-testonly-medium'), 'live list must include the stub-only variant');

  const res = await post(port, { model: 'gemini-9-testonly', messages });
  assert.equal(res.status, 200, 'a model from the live list must be served, not 400ed');
  const body = JSON.parse(await res.text());
  assert.equal(body.model, 'gemini-9-testonly-medium');
  assert.equal(body.choices[0].message.content, 'Hello world');
});

test('live pull failure falls back to the last-known-good catalog; unknowns still 400', async (t) => {
  const { port, stop } = await startProxy({ AGY_STUB_FAIL_MODELS: '1' });
  t.after(stop);
  const messages = [{ role: 'user', content: 'hi' }];

  const health = await (await fetch(`http://127.0.0.1:${port}/health`)).json();
  assert.ok(['cache', 'fallback'].includes(health.models.source), `expected a last-known-good source, got ${health.models.source}`);

  const listRes = await fetch(`http://127.0.0.1:${port}/v1/models`);
  const list = JSON.parse(await listRes.text());
  const ids = list.data.map((m) => m.id);
  assert.ok(ids.includes('gemini-4-argon-medium'), 'fallback catalog must still serve Argon');

  const okRes = await post(port, { model: 'gemini-4-argon', messages });
  assert.equal(okRes.status, 200);
  await okRes.text();

  const badRes = await post(port, { model: 'gemini-9-nonexistent', messages });
  assert.equal(badRes.status, 400, 'unknown ids are 400 against the fallback list too');
  await badRes.text();
});

test('non-object JSON body is a 400 and the proxy stays alive (#2816)', async (t) => {
  const { port, stop } = await startProxy();
  t.after(stop);
  for (const bad of ['null', '[1,2]', '"x"', '42']) {
    const res = await fetch(`http://127.0.0.1:${port}/v1/chat/completions`, {
      method: 'POST', headers: { 'Content-Type': 'application/json' }, body: bad
    });
    assert.equal(res.status, 400, `body ${bad} must be a 400, got ${res.status}`);
    await res.text();
  }
  const alive = await fetch(`http://127.0.0.1:${port}/v1/chat/completions`, {
    method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{bad json'
  });
  assert.equal(alive.status, 400, 'proxy must still serve after non-object bodies');
  await alive.text();
});
