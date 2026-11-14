// Tests for the codedb MCP bridge (#4120).
// Run: node --test bin/codedb-bridge.test.mjs
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath, pathToFileURL } from 'node:url';
import {
  CODEDB_TOOLS,
  createHandler,
  DEFAULT_DAEMON_FETCH_TIMEOUT_MS,
  parseDaemonFetchTimeoutMs,
  parseDaemonUrl,
  routeForTool,
} from './codedb-bridge.mjs';

const binDir = path.dirname(fileURLToPath(import.meta.url));

test('daemon URL defaults to the codedb serve origin and rejects non-origins', () => {
  assert.equal(parseDaemonUrl(undefined), 'http://127.0.0.1:7719');
  assert.equal(parseDaemonUrl(''), 'http://127.0.0.1:7719');
  assert.equal(parseDaemonUrl('http://127.0.0.1:7719/'), 'http://127.0.0.1:7719');
  assert.throws(() => parseDaemonUrl('not a url'), /CODEDB_DAEMON_URL/);
  assert.throws(() => parseDaemonUrl('https://127.0.0.1:7719'), /CODEDB_DAEMON_URL/);
  assert.throws(() => parseDaemonUrl('http://127.0.0.1:7719/explore'), /CODEDB_DAEMON_URL/);
});

test('daemon URL rejects query, fragment and userinfo by name instead of silently stripping them (#4198)', () => {
  assert.throws(() => parseDaemonUrl('http://127.0.0.1:7719/?token=secret'), /CODEDB_DAEMON_URL.*query/);
  assert.throws(() => parseDaemonUrl('http://127.0.0.1:7719/?next=/explore/tree'), /CODEDB_DAEMON_URL.*query/);
  assert.throws(() => parseDaemonUrl('http://127.0.0.1:7719/#frag'), /CODEDB_DAEMON_URL.*fragment/);
  assert.throws(() => parseDaemonUrl('http://user:pass@127.0.0.1:7719/'), /CODEDB_DAEMON_URL.*userinfo/);
  assert.throws(() => parseDaemonUrl('http://user@127.0.0.1:7719'), /CODEDB_DAEMON_URL.*userinfo/);
});

test('daemon URL errors never echo credentials, query values or fragment values (#9829)', () => {
  const messageOf = (fn) => {
    try {
      fn();
    } catch (err) {
      return err.message;
    }
    throw new Error('expected parseDaemonUrl to throw');
  };
  // userinfo branch: neither the username nor the password may appear
  // (distinctive sentinels -- the word userinfo itself contains user).
  const userinfo = messageOf(() => parseDaemonUrl('http://svcacct:hunterpass@127.0.0.1:7719/'));
  assert.match(userinfo, /CODEDB_DAEMON_URL.*userinfo/);
  assert.ok(!userinfo.includes('hunterpass'), userinfo);
  assert.ok(!userinfo.includes('svcacct'), userinfo);
  const userinfoPass = messageOf(() => parseDaemonUrl('http://user:pass@127.0.0.1:7719/'));
  assert.ok(!userinfoPass.includes('pass'), userinfoPass);
  // query branch: the token value may not appear.
  const query = messageOf(() => parseDaemonUrl('http://127.0.0.1:7719/?token=secret'));
  assert.match(query, /CODEDB_DAEMON_URL.*query/);
  assert.ok(!query.includes('secret'), query);
  // fragment branch: the fragment value may not appear.
  const fragment = messageOf(() => parseDaemonUrl('http://127.0.0.1:7719/#secretfrag'));
  assert.match(fragment, /CODEDB_DAEMON_URL.*fragment/);
  assert.ok(!fragment.includes('secretfrag'), fragment);
  // the pre-existing branches are covered too: a credentialed value that
  // fails URL parsing or the origin check must not be echoed either.
  const invalid = messageOf(() => parseDaemonUrl('http://user:pass@127.0.0.1:bad/'));
  assert.ok(!invalid.includes('pass'), invalid);
  const nonOrigin = messageOf(() => parseDaemonUrl('https://user:pass@127.0.0.1:7719'));
  assert.ok(!nonOrigin.includes('pass'), nonOrigin);
});

test('daemon fetch timeout defaults to a named deadline and accepts a positive integer override', () => {
  assert.equal(DEFAULT_DAEMON_FETCH_TIMEOUT_MS, 10_000);
  assert.equal(parseDaemonFetchTimeoutMs(undefined), DEFAULT_DAEMON_FETCH_TIMEOUT_MS);
  assert.equal(parseDaemonFetchTimeoutMs('250'), 250);
  assert.throws(() => parseDaemonFetchTimeoutMs('0'), /positive integer/);
  assert.throws(() => parseDaemonFetchTimeoutMs('nope'), /positive integer/);
});

test('every advertised tool maps to a verified daemon route', () => {
  const args = {
    codedb_tree: {},
    codedb_outline: { path: 'a/b.rs' },
    codedb_symbol: { name: 'IdentityStore' },
    codedb_search: { query: 'IdentityStore' },
    codedb_word: { word: 'IdentityStore' },
    codedb_hot: {},
    codedb_deps: { path: 'a/b.rs' },
  };
  for (const t of CODEDB_TOOLS) {
    const { route } = routeForTool(t.name, args[t.name]);
    assert.ok(route.startsWith('/explore/'), `${t.name} -> ${route}`);
  }
  assert.equal(routeForTool('codedb_search', { query: 'a b' }).route, '/explore/search?q=a%20b');
  assert.equal(routeForTool('codedb_word', { word: 'x' }).route, '/explore/word?q=x');
  assert.equal(routeForTool('codedb_symbol', { name: 'x' }).route, '/explore/symbol?name=x');
});

test('unhonourable arguments are rejected by name, never silently ignored (#3352)', () => {
  assert.throws(() => routeForTool('codedb_search', { query: 'x', regex: true }), /"regex"/);
  assert.throws(() => routeForTool('codedb_search', { query: 'x', scope: true }), /"scope"/);
  assert.throws(() => routeForTool('codedb_outline', { path: 'x', compact: true }), /"compact"/);
  assert.throws(() => routeForTool('codedb_tree', { project: '/elsewhere' }), /"project"/);
  assert.throws(() => routeForTool('codedb_search', {}), /"query" is required/);
  assert.throws(() => routeForTool('codedb_read', { path: 'x' }), /unknown tool "codedb_read"/);
});

test('regex, scope and compact are rejected by name for every advertised tool (#4197)', () => {
  const args = {
    codedb_tree: {},
    codedb_outline: { path: 'a/b.rs' },
    codedb_symbol: { name: 'IdentityStore' },
    codedb_search: { query: 'IdentityStore' },
    codedb_word: { word: 'IdentityStore' },
    codedb_hot: {},
    codedb_deps: { path: 'a/b.rs' },
  };
  for (const { name } of CODEDB_TOOLS) {
    for (const key of ['regex', 'scope', 'compact']) {
      assert.throws(
        () => routeForTool(name, { ...args[name], [key]: true }),
        /argument ".*"/,
        `${name} must reject ${key}: true`
      );
    }
  }
});

test('misnamed limit arguments are rejected instead of silently ignored (#4197)', () => {
  assert.throws(() => routeForTool('codedb_search', { query: 'x', limit: 5 }), /argument "limit"/);
  assert.throws(() => routeForTool('codedb_hot', { max_results: 2 }), /argument "max_results"/);
});

const stubFetch = (routes) => async (url) => {
  const path = url.replace('http://127.0.0.1:7719', '');
  if (!(path in routes)) return { ok: false, status: 404, json: async () => ({ error: 'not found' }) };
  return { ok: true, status: 200, json: async () => routes[path] };
};

const call = async (handle, name, args) =>
  handle({ jsonrpc: '2.0', id: 9, method: 'tools/call', params: { name, arguments: args } });

test('handler answers initialize, tools/list and ping; notifications get no reply', async () => {
  const handle = createHandler({ fetchFn: stubFetch({}), daemonUrl: 'http://127.0.0.1:7719' });
  const init = await handle({ jsonrpc: '2.0', id: 1, method: 'initialize', params: { protocolVersion: '2024-11-05' } });
  assert.equal(init.result.serverInfo.name, 'codedb-bridge');
  assert.equal(init.result.protocolVersion, '2024-11-05');
  const list = await handle({ jsonrpc: '2.0', id: 2, method: 'tools/list' });
  assert.deepEqual(list.result.tools.map((t) => t.name), CODEDB_TOOLS.map((t) => t.name));
  assert.deepEqual((await handle({ jsonrpc: '2.0', id: 3, method: 'ping' })).result, {});
  assert.equal(await handle({ jsonrpc: '2.0', method: 'notifications/initialized' }), null);
  const bad = await handle({ jsonrpc: '2.0', id: 4, method: 'resources/list' });
  assert.equal(bad.error.code, -32601);
});

test('tools/call forwards to the daemon and returns its payload', async () => {
  const handle = createHandler({
    fetchFn: stubFetch({
      '/explore/tree': { tree: 'a/\n  b.rs  rust  3L  1 sym\n' },
      '/explore/search?q=IdentityStore': {
        query: 'IdentityStore',
        results: [
          { path: 'x.rs', line: 1, text: 'a' },
          { path: 'y.rs', line: 2, text: 'b' },
          { path: 'z.rs', line: 3, text: 'c' },
        ],
      },
      '/explore/hot': { files: ['a', 'b', 'c'] },
    }),
    daemonUrl: 'http://127.0.0.1:7719',
  });
  const tree = await call(handle, 'codedb_tree', {});
  assert.equal(tree.result.content[0].text, 'a/\n  b.rs  rust  3L  1 sym\n');
  assert.equal(tree.result.isError, undefined);
  const search = await call(handle, 'codedb_search', { query: 'IdentityStore', max_results: 2 });
  assert.deepEqual(JSON.parse(search.result.content[0].text).results.map((r) => r.path), ['x.rs', 'y.rs']);
  const hot = await call(handle, 'codedb_hot', { limit: 1 });
  assert.deepEqual(JSON.parse(hot.result.content[0].text).files, ['a']);
});

test('arguments in no tool schema are rejected by name, never silently dropped (#4216)', () => {
  const neverNamed = {
    codedb_tree: { bogus_tree_opt: 1 },
    codedb_outline: { path: 'x', bogus_outline_opt: 1 },
    codedb_symbol: { name: 'x', bogus_symbol_opt: 1 },
    codedb_search: { query: 'x', bogus_search_opt: 1 },
    codedb_word: { word: 'x', bogus_word_opt: 1 },
    codedb_hot: { bogus_hot_opt: 1 },
    codedb_deps: { path: 'x', bogus_deps_opt: 1 },
  };
  for (const t of CODEDB_TOOLS) {
    const key = Object.keys(neverNamed[t.name]).find((k) => k.startsWith('bogus_'));
    assert.throws(() => routeForTool(t.name, neverNamed[t.name]), new RegExp(`"${key}"`));
  }
  // The typo case from the issue: max_result (no s) is not max_results.
  assert.throws(() => routeForTool('codedb_search', { query: 'x', max_result: 2 }), /"max_result"/);
});

test('a typoed max_result surfaces as an MCP error naming the argument, not the full result set (#4216)', async () => {
  const handle = createHandler({
    fetchFn: stubFetch({
      '/explore/search?q=x': {
        query: 'x',
        results: [
          { path: 'a.rs', line: 1, text: 'a' },
          { path: 'b.rs', line: 2, text: 'b' },
          { path: 'c.rs', line: 3, text: 'c' },
          { path: 'd.rs', line: 4, text: 'd' },
          { path: 'e.rs', line: 5, text: 'e' },
        ],
      },
    }),
    daemonUrl: 'http://127.0.0.1:7719',
  });
  const typo = await call(handle, 'codedb_search', { query: 'x', max_result: 2 });
  assert.equal(typo.result.isError, true);
  assert.match(typo.result.content[0].text, /"max_result"/);
});

test('a slow daemon call does not head-of-line block a later ping (PR #4171 Codex P2)', async () => {
  const daemon = http.createServer((req, res) => {
    const respond = () => {
      res.setHeader('content-type', 'application/json');
      res.end(JSON.stringify({ query: 'x', results: [] }));
    };
    if (req.url.startsWith('/explore/search')) setTimeout(respond, 1200);
    else respond();
  });
  await new Promise((resolve) => daemon.listen(0, '127.0.0.1', resolve));
  const child = spawn(process.execPath, [path.join(binDir, 'codedb-bridge.mjs')], {
    env: { ...process.env, CODEDB_DAEMON_URL: `http://127.0.0.1:${daemon.address().port}` },
    stdio: ['pipe', 'pipe', 'inherit'],
  });
  try {
    const order = [];
    const done = new Promise((resolve) => {
      let buf = '';
      child.stdout.on('data', (chunk) => {
        buf += chunk;
        for (const line of buf.split('\n')) {
          if (!line.trim()) continue;
          try {
            order.push(JSON.parse(line).id);
          } catch {}
        }
        buf = buf.slice(buf.lastIndexOf('\n') + 1);
        if (order.length >= 2) resolve();
      });
    });
    child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/call', params: { name: 'codedb_search', arguments: { query: 'x' } } })}\n`);
    child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id: 2, method: 'ping' })}\n`);
    await done;
    assert.deepEqual(order, [2, 1], 'ping (id 2) must be answered before the slow search (id 1) completes');
  } finally {
    child.stdin.end();
    child.kill();
    daemon.close();
  }
});

test('a never-resolving daemon fetch returns a timeout MCP error naming URL, route and deadline', async () => {
  const timeoutMs = 25;
  const daemonUrl = 'http://127.0.0.1:7719';
  const handle = createHandler({ fetchFn: () => new Promise(() => {}), daemonUrl, timeoutMs });
  const response = await call(handle, 'codedb_tree', {});
  assert.equal(response.result.isError, true);
  assert.match(response.result.content[0].text, /timed out after 25ms/);
  assert.match(response.result.content[0].text, /http:\/\/127\.0\.0\.1:7719\/explore\/tree/);
});

test('bridge exits after stdin closes while a hanging daemon call times out', async () => {
  const daemon = http.createServer(() => {});
  await new Promise((resolve) => daemon.listen(0, '127.0.0.1', resolve));
  const daemonUrl = `http://127.0.0.1:${daemon.address().port}`;
  const child = spawn(process.execPath, [path.join(binDir, 'codedb-bridge.mjs')], {
    env: { ...process.env, CODEDB_DAEMON_URL: daemonUrl, CODEDB_DAEMON_TIMEOUT_MS: '50' },
    stdio: ['pipe', 'pipe', 'inherit'],
  });
  try {
    let received = '';
    child.stdout.on('data', (chunk) => {
      received += chunk;
    });
    const exited = new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('bridge did not exit after its daemon timeout')), 2_000);
      child.once('exit', (code) => {
        clearTimeout(timer);
        resolve(code);
      });
    });
    child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/call', params: { name: 'codedb_tree', arguments: {} } })}\n`);
    child.stdin.end();
    assert.equal(await exited, 0);
    const response = JSON.parse(received.trim());
    assert.equal(response.id, 1);
    assert.equal(response.result.isError, true);
    assert.match(response.result.content[0].text, /timed out after 50ms/);
    assert.match(response.result.content[0].text, /\/explore\/tree/);
  } finally {
    child.kill();
    daemon.closeAllConnections();
    daemon.close();
  }
});

test('closing stdin mid-request still delivers the complete final response (PR #4171 Codex P2)', async () => {
  const bigTree = `root/\n${'  file.rs  rust  1L  0 sym\n'.repeat(80000)}`;
  const daemon = http.createServer((req, res) => {
    res.setHeader('content-type', 'application/json');
    if (req.url.startsWith('/explore/tree')) setTimeout(() => res.end(JSON.stringify({ tree: bigTree })), 200);
    else res.end(JSON.stringify({}));
  });
  await new Promise((resolve) => daemon.listen(0, '127.0.0.1', resolve));
  const child = spawn(process.execPath, [path.join(binDir, 'codedb-bridge.mjs')], {
    env: { ...process.env, CODEDB_DAEMON_URL: `http://127.0.0.1:${daemon.address().port}` },
    stdio: ['pipe', 'pipe', 'inherit'],
  });
  try {
    let received = '';
    const exited = new Promise((resolve) => child.on('exit', resolve));
    child.stdout.on('data', (chunk) => {
      received += chunk;
    });
    child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/call', params: { name: 'codedb_tree', arguments: {} } })}\n`);
    child.stdin.end(); // close immediately, while the daemon call is still pending
    await exited;
    const msg = JSON.parse(received.trim());
    assert.equal(msg.id, 1);
    assert.equal(msg.result.content[0].text, bigTree, 'final response must arrive complete, not truncated by process.exit');
  } finally {
    child.kill();
    daemon.close();
  }
});

test('CLI: runs main via file and directory symlinks (#4325)', () => {
  const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'codedb-bridge-entrypoint-'));
  const bridge = path.join(binDir, 'codedb-bridge.mjs');
  const fileLink = path.join(temp, 'codedb-bridge-link.mjs');
  const dirLink = path.join(temp, 'bin-link');
  fs.symlinkSync(bridge, fileLink);
  fs.symlinkSync(binDir, dirLink, 'dir');

  try {
    const input = `${JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'ping' })}\n`;
    for (const [label, entry] of [
      ['direct', bridge],
      ['file symlink', fileLink],
      ['directory symlink', path.join(dirLink, 'codedb-bridge.mjs')],
    ]) {
      const result = spawnSync(process.execPath, [entry], {
        input,
        encoding: 'utf8',
        timeout: 5000,
      });
      assert.equal(result.status, 0, `${label}: ${result.stderr}`);
      assert.notEqual(result.stdout, '', `${label}: main did not write a response`);
      assert.deepEqual(JSON.parse(result.stdout), {
        jsonrpc: '2.0', id: 1, result: {},
      }, `${label} must start the stdio bridge`);
    }
  } finally {
    fs.rmSync(temp, { recursive: true, force: true });
  }
});

test('importing codedb-bridge.mjs as a library does not start main (#4325)', () => {
  const bridge = path.join(binDir, 'codedb-bridge.mjs');
  const result = spawnSync(process.execPath, [
    '--input-type=module', '-e', `await import(${JSON.stringify(pathToFileURL(bridge).href)});`,
  ], { encoding: 'utf8', timeout: 5000 });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stdout, '');
  assert.equal(result.stderr, '');
});

test('daemon errors and an unreachable daemon surface as MCP errors naming the cause', async () => {
  const handle404 = createHandler({ fetchFn: stubFetch({}), daemonUrl: 'http://127.0.0.1:7719' });
  const missing = await call(handle404, 'codedb_outline', { path: 'nope.rs' });
  assert.equal(missing.result.isError, true);
  assert.match(missing.result.content[0].text, /not found/);
  const down = createHandler({
    fetchFn: async () => {
      throw new Error('connect ECONNREFUSED');
    },
    daemonUrl: 'http://127.0.0.1:7719',
  });
  const unreachable = await call(down, 'codedb_tree', {});
  assert.equal(unreachable.result.isError, true);
  assert.match(unreachable.result.content[0].text, /unreachable.*systemctl --user start codedb-/);
  const badArg = await call(down, 'codedb_search', { query: 'x', regex: true });
  assert.equal(badArg.result.isError, true);
  assert.match(badArg.result.content[0].text, /"regex"/);
});

test('null or non-object daemon bodies are isError results, never a throw or a "null" success (#6917)', async () => {
  const handle = createHandler({
    fetchFn: stubFetch({
      '/explore/tree': null,
      '/explore/search?q=x': null,
      '/explore/outline?path=a.ts': 'weird',
      '/explore/hot': [1, 2],
    }),
    daemonUrl: 'http://127.0.0.1:7719',
  });
  const tree = await call(handle, 'codedb_tree', {});
  assert.equal(tree.result.isError, true);
  assert.match(tree.result.content[0].text, /null JSON body for \/explore\/tree/);
  const search = await call(handle, 'codedb_search', { query: 'x' });
  assert.equal(search.result.isError, true);
  assert.match(search.result.content[0].text, /null JSON body/);
  const outline = await call(handle, 'codedb_outline', { path: 'a.ts' });
  assert.equal(outline.result.isError, true);
  assert.match(outline.result.content[0].text, /string JSON body/);
  const hot = await call(handle, 'codedb_hot', {});
  assert.equal(hot.result.isError, true);
  assert.match(hot.result.content[0].text, /array JSON body/);
});
