import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync, utimesSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

import {
  attemptViolations,
  credentialViolations,
  inspectSeating,
  parseArgs,
  seatedGoLeaves,
} from './check-opencode-go-seating.mjs';

const require = createRequire(import.meta.url);

// The popesdrew shape (#5611): the weighted pool config is CORRECT — both Go
// leaves are seated as peers — but the credential was provisioned under the
// bare console-login key `opencode`, so the exact-id selector finds nothing.
const seatedPoolConfig = `{
  "provider": {
    "opencode-go.drewrypope": { "instanceOf": "opencode-go" },
    "opencode-go.popesdrew": { "instanceOf": "opencode-go" },
  },
  "experimental": {
    "provider_routes": {
      "opencode-go-plans": {
        "providers": ["opencode-go.drewrypope", "opencode-go.popesdrew"],
        "selection": "weighted",
        "weights": { "opencode-go.drewrypope": 2, "opencode-go.popesdrew": 1 },
      },
    },
  },
}`;

const NOW = Date.parse('2026-11-04T09:00:00Z');
const HOUR = 3600 * 1000;

function store({ credentialRows, attempts }) {
  const attemptsByProvider = new Map();
  for (const [provider, rows] of Object.entries(attempts ?? {})) {
    attemptsByProvider.set(provider, rows);
  }
  return { credentialRows, attemptsByProvider };
}

const credential = (integrationId, active = true) => ({
  credentialId: `cred-${integrationId}`,
  integrationId,
  label: integrationId,
  active,
});

test('the popesdrew failure shape fails: bare-key credential, correct pool config', () => {
  // This is issue #5611 exactly: pool config correct as deployed, credential
  // under `opencode`, nothing serves the leaf.
  const report = inspectSeating({
    config: JSON.parse(JSON.stringify({ experimental: { provider_routes: {
      'opencode-go-plans': { providers: ['opencode-go.drewrypope', 'opencode-go.popesdrew'] },
    } } })),
    store: store({
      credentialRows: [credential('opencode'), credential('opencode-go.drewrypope')],
      attempts: {
        'opencode-go.drewrypope': [{ outcome: 'success', timeStarted: NOW - HOUR }],
      },
    }),
    stage: 'pre-seat',
    now: NOW,
  });
  assert.equal(report.violations.length, 1);
  assert.match(report.violations[0], /opencode-go\.popesdrew: no credential row exists under the exact instance id/);
  assert.match(report.violations[0], /bare "opencode" console-login row exists/);
});

test('exact-id active credential passes the credential assertion', () => {
  const rows = [credential('opencode'), credential('opencode-go.popesdrew')];
  assert.deepEqual(credentialViolations('opencode-go.popesdrew', rows), []);
});

test('an exact-id credential that is inactive does not seat the leaf', () => {
  const rows = [credential('opencode'), credential('opencode-go.popesdrew', false)];
  const violations = credentialViolations('opencode-go.popesdrew', rows);
  assert.equal(violations.length, 1);
  assert.match(violations[0], /none is active/);
});

test('a successful attempt inside the window passes; outside does not', () => {
  const leaf = 'opencode-go.popesdrew';
  const fresh = [{ outcome: 'success', timeStarted: NOW - 2 * HOUR }];
  const stale = [{ outcome: 'success', timeStarted: NOW - 48 * HOUR }];
  const failing = [
    { outcome: 'failure', timeStarted: NOW - HOUR, errorTag: 'QuotaExceeded' },
    { outcome: 'failure', timeStarted: NOW - 2 * HOUR, errorTag: 'QuotaExceeded' },
  ];
  const opts = { now: NOW, windowHours: 24 };
  assert.deepEqual(attemptViolations(leaf, fresh, opts), []);
  assert.match(attemptViolations(leaf, stale, opts)[0], /no successful route attempt within 24h/);
  assert.match(
    attemptViolations(leaf, fresh, { ...opts, configUpdatedAt: NOW - HOUR })[0],
    /after the active config's latest modification/,
  );
  assert.match(attemptViolations(leaf, failing, opts)[0], /error_tag=QuotaExceeded/);
  assert.match(attemptViolations(leaf, [], opts)[0], /no attempt recorded at all/);
});

test('the second assertion is not optional: seated leaf with a credential but no attempt fails', () => {
  const report = inspectSeating({
    config: { experimental: { provider_routes: {
      'opencode-go-plans': { providers: ['opencode-go.popesdrew'] },
    } } },
    store: store({
      credentialRows: [credential('opencode-go.popesdrew')],
      attempts: {},
    }),
    stage: 'periodic',
    now: NOW,
  });
  assert.equal(report.violations.length, 1);
  assert.match(report.violations[0], /no attempt recorded at all/);
});

test('pre-seat stage skips the attempt assertion (a new leaf has no attempts yet)', () => {
  const report = inspectSeating({
    config: { experimental: { provider_routes: {
      'opencode-go-plans': { providers: ['opencode-go.popesdrew'] },
    } } },
    store: store({ credentialRows: [credential('opencode-go.popesdrew')], attempts: {} }),
    stage: 'pre-seat',
    now: NOW,
  });
  assert.deepEqual(report.violations, []);
});

test('both assertions pass for a healthy seated pair', () => {
  const report = inspectSeating({
    config: JSON.parse(JSON.stringify({ experimental: { provider_routes: {
      'opencode-go-plans': { providers: ['opencode-go.drewrypope', 'opencode-go.popesdrew'] },
    } } })),
    store: store({
      credentialRows: [credential('opencode'), credential('opencode-go.drewrypope'), credential('opencode-go.popesdrew')],
      attempts: {
        'opencode-go.drewrypope': [{ outcome: 'success', timeStarted: NOW - HOUR }],
        'opencode-go.popesdrew': [{ outcome: 'success', timeStarted: NOW - HOUR }],
      },
    }),
    stage: 'periodic',
    now: NOW,
  });
  assert.deepEqual(report.violations, []);
  assert.deepEqual(report.leaves.map((entry) => entry.leaf), ['opencode-go.drewrypope', 'opencode-go.popesdrew']);
});

test('seating is membership, not mention: a provider block or catalog entry alone is not a seat', () => {
  const config = {
    provider: {
      'opencode-go.popesdrew': { instanceOf: 'opencode-go' },
    },
    model_routes: { some: { targets: ['opencode-go.popesdrew/deepseek-v4.1-flash'] } },
  };
  const seats = seatedGoLeaves(config);
  assert.deepEqual([...seats.keys()], ['opencode-go.popesdrew']);
  assert.deepEqual([...seats.get('opencode-go.popesdrew')], ['$.model_routes.some']);

  const unseated = seatedGoLeaves({ provider: { 'opencode-go.popesdrew': {} } });
  assert.equal(unseated.size, 0);
});

test('provider-route group names that look like definition fields are still groups', () => {
  const seats = seatedGoLeaves({
    experimental: { provider_routes: { providers: { providers: ['opencode-go.popesdrew'] } } },
  });
  assert.deepEqual([...seats.keys()], ['opencode-go.popesdrew']);
});

test('parseArgs: bare positional is the config, --skip-attempts means pre-seat', () => {
  const options = parseArgs(['.opencode/opencode.jsonc', '--leaf', 'opencode-go.popesdrew', '--skip-attempts', '--json']);
  assert.equal(options.config, '.opencode/opencode.jsonc');
  assert.equal(options.stage, 'pre-seat');
  assert.equal(options.json, true);
  assert.deepEqual(options.leafIds, ['opencode-go.popesdrew']);
  assert.throws(() => parseArgs(['--stage', 'nonsense']), /--stage/);
  assert.throws(() => parseArgs(['--window-hours', '0']), /--window-hours/);
  assert.throws(() => parseArgs(['--stage', 'pre-seat']), /requires at least one --leaf/);
});

// --- end-to-end: the real binary against a fixture sqlite store ------------

const guardPath = fileURLToPath(new URL('./check-opencode-go-seating.mjs', import.meta.url));

function makeStore({ credentialRows, attempts }) {
  const dir = mkdtempSync(join(tmpdir(), 'go-seating-'));
  const dbPath = join(dir, 'opencode.db');
  const { DatabaseSync } = require('node:sqlite');
  const db = new DatabaseSync(dbPath);
  try {
    db.exec(
      'CREATE TABLE credential (id TEXT PRIMARY KEY, integration_id TEXT, label TEXT NOT NULL, active INTEGER)',
    );
    db.exec(
      'CREATE TABLE route_attempt (id INTEGER PRIMARY KEY AUTOINCREMENT, time_started INTEGER NOT NULL, ' +
        'provider_id TEXT NOT NULL, outcome TEXT NOT NULL, error_tag TEXT)',
    );
    const insertCredential = db.prepare('INSERT INTO credential VALUES (?, ?, ?, ?)');
    for (const row of credentialRows) {
      insertCredential.run(row.credentialId, row.integrationId, row.label, row.active ? 1 : 0);
    }
    const insertAttempt = db.prepare('INSERT INTO route_attempt (time_started, provider_id, outcome, error_tag) VALUES (?, ?, ?, ?)');
    for (const [provider, rows] of Object.entries(attempts ?? {})) {
      for (const attempt of rows) {
        insertAttempt.run(attempt.timeStarted, provider, attempt.outcome, attempt.errorTag ?? null);
      }
    }
  } finally {
    db.close();
  }
  return { dir, dbPath };
}

function runGuard(args) {
  return spawnSync(process.execPath, [guardPath, ...args], { encoding: 'utf8' });
}

function writeConfig(dir, source) {
  const configPath = join(dir, 'opencode.jsonc');
  writeFileSync(configPath, source);
  const beforeSeating = new Date(Date.now() - 2 * HOUR);
  utimesSync(configPath, beforeSeating, beforeSeating);
  return configPath;
}

test('end-to-end: the popesdrew store fails the check with exit 1', () => {
  const { dir, dbPath } = makeStore({
    credentialRows: [credential('opencode'), credential('opencode-go.drewrypope')],
    attempts: { 'opencode-go.drewrypope': [{ outcome: 'success', timeStarted: Date.now() - HOUR }] },
  });
  const configPath = writeConfig(dir, seatedPoolConfig);
  try {
    const res = runGuard([configPath, '--db', dbPath]);
    assert.equal(res.status, 1, res.stderr);
    assert.match(res.stderr, /opencode-go\.popesdrew: no credential row exists under the exact instance id/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('end-to-end: exact-id credentials plus fresh successes pass with exit 0', () => {
  const { dir, dbPath } = makeStore({
    credentialRows: [credential('opencode-go.drewrypope'), credential('opencode-go.popesdrew')],
    attempts: {
      'opencode-go.drewrypope': [{ outcome: 'success', timeStarted: Date.now() - HOUR }],
      'opencode-go.popesdrew': [{ outcome: 'success', timeStarted: Date.now() - HOUR }],
    },
  });
  const configPath = writeConfig(dir, seatedPoolConfig);
  try {
    const res = runGuard([configPath, '--db', dbPath]);
    assert.equal(res.status, 0, res.stderr);
    assert.match(res.stdout, /Go instance seating guard passed/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('end-to-end: a missing store is a loud check error (exit 2), not a silent pass', () => {
  const dir = mkdtempSync(join(tmpdir(), 'go-seating-missing-'));
  const configPath = writeConfig(dir, seatedPoolConfig);
  try {
    const res = runGuard([configPath, '--db', join(dir, 'nope.db')]);
    assert.equal(res.status, 2);
    assert.match(res.stderr, /credential store not found/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('pre-seat registration can check a new leaf before it is added to any pool', () => {
  const { dir, dbPath } = makeStore({
    credentialRows: [credential('opencode-go.popesdrew')],
    attempts: {},
  });
  const configPath = writeConfig(dir, '{"provider": {"opencode-go.popesdrew": {"instanceOf": "opencode-go"}}}');
  try {
    const res = runGuard([configPath, '--db', dbPath, '--leaf', 'opencode-go.popesdrew', '--stage', 'pre-seat']);
    assert.equal(res.status, 0, res.stderr);
    assert.match(res.stdout, /credential rows only \(pre-seat\)/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
