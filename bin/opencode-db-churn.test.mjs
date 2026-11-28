import { test } from 'node:test';
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';
import {
  parseArgs,
  parseDiffSummary,
  toBytes,
  UsageError,
  USAGE,
} from './opencode-db-churn.mjs';

// Every automated test names its oracle. Oracle for the parser tests:
// the declared contract in USAGE plus the strict-parsing rules in the
// header comment. Oracle for parseDiffSummary: hand-computed values from
// a real `restic diff` tail summary captured from the opencode-db repo
// (2026-11-24, snapshot f4a9c52d -> 93937176).

const REAL_DIFF_TAIL = `comparing snapshot f4a9c52d to 93937176:

M    /home/user/backups/opencode-db-staging/opencode.db

Files:           0 new,     0 removed,     1 changed
Dirs:            0 new,     0 removed
Others:          0 new,     0 removed
Data Blobs:  38773 new, 35508 removed
Tree Blobs:      5 new,     5 removed
  Added:   20.094 GiB
  Removed: 18.433 GiB`;

test('parseArgs extracts the diff subcommand with its two ids', () => {
  const { options, flags, positionals } = parseArgs(['diff', 'aaa', 'bbb']);
  assert.deepEqual(positionals, ['diff', 'aaa', 'bbb']);
  assert.deepEqual(options, {});
  assert.equal(flags.size, 0);
});

test('parseArgs accepts --opt value and --opt=value identically', () => {
  const a = parseArgs(['diff', 'x', 'y', '--repo', '/r']);
  const b = parseArgs(['diff', 'x', 'y', '--repo=/r']);
  assert.equal(a.options['--repo'], '/r');
  assert.equal(b.options['--repo'], '/r');
});

test('parseArgs rejects an unknown flag naming it', () => {
  assert.throws(() => parseArgs(['diff', 'x', 'y', '--apend']), (err) => {
    assert.ok(err instanceof UsageError);
    assert.match(err.message, /--apend/);
    return true;
  });
});

test('parseArgs rejects a typoed value option naming it', () => {
  assert.throws(() => parseArgs(['--reop', '/r', 'diff', 'x', 'y']), (err) => {
    assert.ok(err instanceof UsageError);
    assert.match(err.message, /--reop/);
    return true;
  });
});

test('parseArgs rejects a present-but-valueless option', () => {
  assert.throws(() => parseArgs(['diff', 'x', 'y', '--repo']), (err) => {
    assert.ok(err instanceof UsageError);
    assert.match(err.message, /--repo requires a value/);
    return true;
  });
  assert.throws(() => parseArgs(['diff', 'x', 'y', '--repo', '--append']), (err) => {
    assert.ok(err instanceof UsageError);
    assert.match(err.message, /--repo requires a value/);
    return true;
  });
});

test('parseArgs rejects an empty --opt= value', () => {
  assert.throws(() => parseArgs(['diff', 'x', 'y', '--repo=']), (err) => {
    assert.ok(err instanceof UsageError);
    assert.match(err.message, /non-empty/);
    return true;
  });
});

test('parseArgs rejects a repeated option', () => {
  assert.throws(() => parseArgs(['diff', 'x', 'y', '--repo', '/a', '--repo', '/b']), (err) => {
    assert.ok(err instanceof UsageError);
    assert.match(err.message, /more than once/);
    return true;
  });
});

test('parseArgs rejects a value on a boolean flag', () => {
  assert.throws(() => parseArgs(['diff', 'x', 'y', '--append=yes']), (err) => {
    assert.ok(err instanceof UsageError);
    assert.match(err.message, /takes no value/);
    return true;
  });
});

test('parseArgs accepts declared flags', () => {
  const { flags } = parseArgs(['diff', 'x', 'y', '--append', '--json']);
  assert.ok(flags.has('--append'));
  assert.ok(flags.has('--json'));
});

test('parseDiffSummary returns hand-verified numbers from real output', () => {
  const s = parseDiffSummary(REAL_DIFF_TAIL);
  assert.equal(s.dataBlobsNew, 38773);
  assert.equal(s.dataBlobsRemoved, 35508);
  assert.equal(s.addedBytes, Math.round(20.094 * 1024 ** 3));
  assert.equal(s.removedBytes, Math.round(18.433 * 1024 ** 3));
});

test('parseDiffSummary fails loudly on unparseable output, never zeros', () => {
  assert.throws(() => parseDiffSummary('Files: 1 new\nno summary here\n'), (err) => {
    assert.ok(err instanceof UsageError);
    return true;
  });
});

test('toBytes covers every restic unit', () => {
  assert.equal(toBytes(1, 'B'), 1);
  assert.equal(toBytes(2, 'KB'), 2000);
  assert.equal(toBytes(2, 'MB'), 2_000_000);
  assert.equal(toBytes(1.5, 'GB'), 1_500_000_000);
  assert.equal(toBytes(2, 'KiB'), 2048);
  assert.equal(toBytes(2, 'MiB'), 2 * 1024 ** 2);
  assert.equal(toBytes(2, 'GiB'), 2 * 1024 ** 3);
  assert.equal(toBytes(1, 'TiB'), 1024 ** 4);
  assert.throws(() => toBytes(1, 'PB'), UsageError);
});

test('USAGE names every subcommand and option', () => {
  for (const token of ['diff', 'ledger', 'info', '--repo', '--ledger', '--json', '--append']) {
    assert.ok(USAGE.includes(token), `USAGE missing ${token}`);
  }
});

test('importing opencode-db-churn.mjs as a library does not run main (#2235)', async () => {
  const cli = new URL('./opencode-db-churn.mjs', import.meta.url).pathname;
  const { spawnSync } = await import('node:child_process');
  const res = spawnSync(process.execPath, ['--input-type=module', '-e', `import ${JSON.stringify(pathToFileURL(cli).href)};`], {
    encoding: 'utf8',
  });
  assert.equal(res.status, 0);
  assert.equal(res.stdout, '');
});
