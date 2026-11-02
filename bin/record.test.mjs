// Tests for the session-recording CLI (#1853).
// Run: node --test bin/record.test.mjs
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath, pathToFileURL } from 'node:url';
import {
  appendLine,
  normaliseInput,
  parseLoose,
  quarantinePathFor,
  record,
  resolveTarget
} from './record.mjs';

const binDir = path.dirname(fileURLToPath(import.meta.url));
const recordCli = path.join(binDir, 'record.mjs');

const tmpTarget = () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'record-test-'));
  return path.join(dir, 'nested', 'ai_recordings.jsonl');
};

const readLines = (file) =>
  fs
    .readFileSync(file, 'utf8')
    .split('\n')
    .filter((line) => line !== '');

const stamps = { ts: '2026-10-28T00:00:00.000Z', pwd: '/somewhere' };

test('resolveTarget honours AI_RECORDINGS_FILE and falls back to the default', () => {
  assert.equal(resolveTarget({ AI_RECORDINGS_FILE: '/x/y.jsonl' }), '/x/y.jsonl');
  assert.equal(resolveTarget({ AI_RECORDINGS_FILE: '  ' }), resolveTarget({}));
  assert.ok(resolveTarget({}).endsWith(path.join('git', 'ai_recordings.jsonl')));
});

test('round-trip: agent-supplied fields read back byte-identical', () => {
  const target = tmpTarget();
  const supplied = {
    kind: 'tool-lied',
    sentiment: { positive: ['measured instead'], negative: ['framing'] },
    note: 'The verifier had five bugs.',
    grade: 'F',
    count: 117,
    ratio: 1.5,
    fixed: true,
    nothing: null,
    unknown_key: ['any', { shape: 'goes' }]
  };
  const outcome = record(JSON.stringify(supplied), {
    target,
    cwd: '/nonexistent-dir',
    env: {},
    now: new Date('2026-10-28T12:00:00.000Z')
  });
  assert.equal(outcome, 'recorded');
  const lines = readLines(target);
  assert.equal(lines.length, 1);
  const entry = JSON.parse(lines[0]);
  for (const [key, value] of Object.entries(supplied)) {
    assert.equal(
      JSON.stringify(entry[key]),
      JSON.stringify(value),
      `field ${key} must round-trip byte-identical`
    );
  }
  // Stamps are filled in for keys the agent did not supply.
  assert.equal(entry.ts, '2026-10-28T12:00:00.000Z');
  assert.equal(entry.pwd, '/nonexistent-dir');
});

test('stamps never overwrite agent-supplied fields', () => {
  const { entry } = normaliseInput(
    JSON.stringify({ ts: 'agent-ts', pwd: '/agent', sentiment: 'good' }),
    stamps
  );
  assert.equal(entry.ts, 'agent-ts');
  assert.equal(entry.pwd, '/agent');
  assert.equal(entry.sentiment, 'good');
});

test('sentiment passes through in any shape, untouched', () => {
  for (const sentiment of [
    'good',
    ['tag1', 'tag2'],
    { fixed: true },
    { positive: ['a'], negative: ['b'] },
    42
  ]) {
    const { entry } = normaliseInput(JSON.stringify({ sentiment }), stamps);
    assert.deepEqual(entry.sentiment, sentiment);
  }
});

test('the target file is created on first append, parents included', () => {
  const target = tmpTarget();
  assert.ok(!fs.existsSync(target));
  assert.equal(record('{"kind":"first"}', { target, env: {} }), 'recorded');
  assert.ok(fs.existsSync(target));
  assert.equal(readLines(target).length, 1);
});

test('non-object JSON is wrapped as a note, not rejected', () => {
  assert.deepEqual(normaliseInput('"just a thought"', stamps).entry.note, 'just a thought');
  assert.deepEqual(normaliseInput('[1,2]', stamps).entry.note, [1, 2]);
});

test('loose structured text normalises when two or more keyed segments parse', () => {
  const loose = parseLoose(
    'category=bad judgement | finding: the stub modelled it as one budget | result: rework {grade: F}'
  );
  assert.equal(loose.category, 'bad judgement');
  assert.equal(loose.finding, 'the stub modelled it as one budget');
  assert.equal(loose.result, 'rework {grade: F}');
  const { entry, quarantine } = normaliseInput(
    'kind=mistake | note: I said 117, then 116 — both stale',
    stamps
  );
  assert.equal(quarantine, undefined);
  assert.equal(entry.kind, 'mistake');
  assert.match(entry.note, /117/);
  assert.equal(entry.raw, 'kind=mistake | note: I said 117, then 116 — both stale');
});

test('unparseable input is quarantined verbatim and still succeeds', () => {
  const target = tmpTarget();
  const raw = 'this is {not json at all, just a feeling';
  const outcome = record(raw, { target, env: {} });
  assert.equal(outcome, 'quarantined');
  assert.ok(!fs.existsSync(target), 'no entry line is written');
  assert.equal(fs.readFileSync(quarantinePathFor(target), 'utf8'), `${raw}\n`);
});

test('a single keyed segment is not confidently structured: quarantined', () => {
  const { entry, quarantine } = normaliseInput('note: one pair only', stamps);
  assert.equal(entry, undefined);
  assert.equal(quarantine, 'note: one pair only');
});

test('empty input is a no-op', () => {
  const target = tmpTarget();
  assert.equal(record('   ', { target, env: {} }), 'noop');
  assert.ok(!fs.existsSync(target));
});

test('an unwritable target is a silent no-op, never a throw', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'record-test-'));
  const blocker = path.join(dir, 'blocker');
  fs.writeFileSync(blocker, 'a file, not a directory');
  const target = path.join(blocker, 'ai_recordings.jsonl');
  assert.equal(record('{"kind":"x"}', { target, env: {} }), 'noop');
  assert.equal(record('total garbage {{{', { target, env: {} }), 'noop');
});

test('appendLine writes the whole line in one append', () => {
  const target = tmpTarget();
  appendLine(target, '{"a":1}');
  appendLine(target, '{"b":2}');
  assert.deepEqual(readLines(target), ['{"a":1}', '{"b":2}']);
});

test('CLI: argument, stdin, quarantine and unwritable all exit 0', () => {
  const target = tmpTarget();
  const env = { ...process.env, AI_RECORDINGS_FILE: target };
  const byArg = spawnSync(process.execPath, [recordCli, '{"kind":"via-arg"}'], {
    env,
    encoding: 'utf8'
  });
  assert.equal(byArg.status, 0, byArg.stderr);
  const byStdin = spawnSync(process.execPath, [recordCli], {
    env,
    input: '{"kind":"via-stdin"}',
    encoding: 'utf8'
  });
  assert.equal(byStdin.status, 0, byStdin.stderr);
  const garbage = spawnSync(process.execPath, [recordCli, 'not json {{{'], {
    env,
    encoding: 'utf8'
  });
  assert.equal(garbage.status, 0, garbage.stderr);
  const entries = readLines(target).map((line) => JSON.parse(line));
  assert.deepEqual(
    entries.map((entry) => entry.kind),
    ['via-arg', 'via-stdin']
  );
  assert.equal(
    fs.readFileSync(quarantinePathFor(target), 'utf8'),
    'not json {{{\n'
  );
});

test('CLI: runs main via file symlink and via directory symlink (#2235)', () => {
  // Regression: the entry-point guard compared the raw invoked path with
  // the resolved module URL, so a symlinked invocation silently skipped
  // main and recorded nothing while exiting 0.
  const target = tmpTarget();
  const env = { ...process.env, AI_RECORDINGS_FILE: target };
  const linkDir = fs.mkdtempSync(path.join(os.tmpdir(), 'record-link-'));

  const fileLink = path.join(linkDir, 'record-link.mjs');
  fs.symlinkSync(recordCli, fileLink);
  const byFile = spawnSync(process.execPath, [fileLink, '{"kind":"via-file-symlink"}'], {
    env,
    encoding: 'utf8'
  });
  assert.equal(byFile.status, 0, byFile.stderr);

  const dirLink = path.join(linkDir, 'bin-link');
  fs.symlinkSync(binDir, dirLink, 'dir');
  const byDir = spawnSync(process.execPath, [path.join(dirLink, 'record.mjs'), '{"kind":"via-dir-symlink"}'], {
    env,
    encoding: 'utf8'
  });
  assert.equal(byDir.status, 0, byDir.stderr);

  const entries = readLines(target).map((line) => JSON.parse(line));
  assert.deepEqual(
    entries.map((entry) => entry.kind),
    ['via-file-symlink', 'via-dir-symlink']
  );
});

test('importing record.mjs as a library does not run main (#2235)', () => {
  const target = tmpTarget();
  const env = { ...process.env, AI_RECORDINGS_FILE: target };
  const res = spawnSync(
    process.execPath,
    ['--input-type=module', '-e', `import ${JSON.stringify(pathToFileURL(recordCli).href)};`],
    { env, encoding: 'utf8' }
  );
  assert.equal(res.status, 0, res.stderr);
  assert.equal(fs.existsSync(target), false);
});

test('concurrent CLI appends produce only whole lines', async () => {
  const target = tmpTarget();
  const env = { ...process.env, AI_RECORDINGS_FILE: target };
  const writers = 12;
  await Promise.all(
    Array.from({ length: writers }, (_, i) =>
      new Promise((resolve, reject) => {
        const child = spawn(
          process.execPath,
          [recordCli, JSON.stringify({ kind: 'concurrent', writer: i })],
          { env, stdio: 'ignore' }
        );
        child.on('exit', (code) =>
          code === 0 ? resolve() : reject(new Error(`writer ${i} exited ${code}`))
        );
        child.on('error', reject);
      })
    )
  );
  const lines = readLines(target);
  assert.equal(lines.length, writers);
  const seen = new Set();
  for (const line of lines) {
    const entry = JSON.parse(line); // throws on any interleaved partial line
    assert.equal(entry.kind, 'concurrent');
    seen.add(entry.writer);
  }
  assert.equal(seen.size, writers);
});
