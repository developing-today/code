// Probe matrix for the bin/ home-resolution helper (#11205, contract #5762).
// Run: node --test bin/home-dir.test.mjs
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { expandTilde, homePath, HomeUnsetError, resolveHome, xdgStateHome } from './home-dir.mjs';

const binDir = path.dirname(fileURLToPath(import.meta.url));
const unsetOrEmpty = [{}, { HOME: '' }];
const named = (purpose) => (err) =>
  err instanceof HomeUnsetError && err.message.startsWith(`HOME is not set or is empty: ${purpose} requires a non-empty HOME`);

test('HOME set resolves to exactly HOME', () => {
  assert.equal(resolveHome({ HOME: '/home/a' }), '/home/a');
  assert.equal(homePath({ HOME: '/home/a' }, 'x', '.local', 'state'), '/home/a/.local/state');
});

test('HOME unset or empty is a named error, never a substitute directory', () => {
  for (const env of unsetOrEmpty) {
    assert.throws(() => resolveHome(env, 'probe'), named('probe'));
    assert.throws(() => homePath(env, 'probe', 'git'), named('probe'));
    assert.throws(() => expandTilde('~/x', env, 'probe'), named('probe'));
    assert.throws(() => xdgStateHome(env, 'probe'), named('probe'));
  }
});

test('tilde expansion only touches ~ and ~/', () => {
  const env = { HOME: '/home/a' };
  assert.equal(expandTilde('~', env), '/home/a');
  assert.equal(expandTilde('~/x/y', env), '/home/a/x/y');
  for (const raw of ['/abs', 'rel', '~other/x', 'a~/b', '']) assert.equal(expandTilde(raw, {}), raw);
});

test('an absolute XDG_STATE_HOME wins; a relative or empty one is ignored', () => {
  assert.equal(xdgStateHome({ HOME: '/home/a', XDG_STATE_HOME: '/xdg' }), '/xdg');
  assert.equal(xdgStateHome({ XDG_STATE_HOME: '/xdg' }), '/xdg');
  for (const XDG_STATE_HOME of ['', 'relative/state', './state']) {
    assert.equal(xdgStateHome({ HOME: '/home/a', XDG_STATE_HOME }), '/home/a/.local/state');
    assert.throws(() => xdgStateHome({ XDG_STATE_HOME }, 'probe'), named('probe'));
  }
});

// The proxies resolve their default paths at startup, before any read,
// write, spawn or connect: with HOME unset they exit naming HOME.
test('opencode-route-proxy refuses to start without HOME', () => {
  for (const env of unsetOrEmpty) {
    const run = spawnSync(process.execPath, [path.join(binDir, 'opencode-route-proxy.mjs')], {
      env: { PATH: process.env.PATH, ...env },
      encoding: 'utf8',
      timeout: 20000,
    });
    assert.notEqual(run.status, 0);
    assert.match(run.stderr, /HOME is not set or is empty: opencode-route-proxy/);
  }
});
