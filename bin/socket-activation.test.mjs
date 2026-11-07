// Tests for systemd socket activation in the provider proxies (#1821).
// Run: node --test bin/socket-activation.test.mjs
//
// Unit level: the LISTEN_FDS/LISTEN_PID parsing in socket-activation.mjs.
// The integration level (spawning a proxy the way systemd starts it,
// with the listening socket handed over as fd 3) retired with the
// codex proxy in the 2026-11-06 proxy-infra decommission: it was
// the last socket-activated door.
import assert from 'node:assert/strict';
import test from 'node:test';
import { listenTarget, systemdActivationFd } from './socket-activation.mjs';

test('systemdActivationFd parses the LISTEN_FDS protocol', () => {
  assert.equal(systemdActivationFd({}, 123), null);
  assert.equal(systemdActivationFd({ LISTEN_FDS: '0' }, 123), null);
  assert.equal(systemdActivationFd({ LISTEN_FDS: 'x' }, 123), null);
  assert.equal(systemdActivationFd({ LISTEN_FDS: '1' }, 123), 3);
  assert.equal(systemdActivationFd({ LISTEN_FDS: '2' }, 123), 3);
  assert.equal(systemdActivationFd({ LISTEN_FDS: '1', LISTEN_PID: '123' }, 123), 3);
  // An environment leaked from another process's activation is ignored.
  assert.equal(systemdActivationFd({ LISTEN_FDS: '1', LISTEN_PID: '999' }, 123), null);
});

test('listenTarget binds directly without activation, uses fd 3 with it', () => {
  assert.deepEqual(listenTarget({ port: 18742, host: '127.0.0.1' }, {}, 123), { port: 18742, host: '127.0.0.1' });
  assert.deepEqual(listenTarget({ port: 18742, host: '127.0.0.1' }, { LISTEN_FDS: '1' }, 123), { fd: 3 });
});
