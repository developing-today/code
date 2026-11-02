// Tests for sync-models' parseJsonc config reader (#704).
// Run: node --test bin/sync-models.test.mjs
//
// sync-models.mjs is a top-level script (it runs the sync on import), so
// the function under test is extracted from the source and evaluated —
// the same verbatim-extraction method that reproduced #704 in triage.
// If parseJsonc's declaration shape changes, this extraction fails loudly
// rather than silently testing nothing.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const binDir = path.dirname(fileURLToPath(import.meta.url));
const source = fs.readFileSync(path.join(binDir, 'sync-models.mjs'), 'utf-8');

const start = source.indexOf('function parseJsonc(text) {');
assert.notEqual(start, -1, 'parseJsonc not found in sync-models.mjs');
const end = source.indexOf('\n}\n', start);
assert.notEqual(end, -1, 'parseJsonc body not terminated as expected');
const parseJsonc = new Function(
  `${source.slice(start, end + 3)}\nreturn parseJsonc;`,
)();

test('parses a config with a multi-line block comment', () => {
  const config = parseJsonc(`{
    /* multi
       line
       comment */
    "a": 1
  }`);
  assert.deepEqual(config, { a: 1 });
});

test('parses a string containing " // " verbatim', () => {
  const config = parseJsonc(`{
    "url": "https://example.com // path",
    "a": 1
  }`);
  assert.equal(config.url, 'https://example.com // path');
  assert.equal(config.a, 1);
});

test('parses a string containing ", ]" without corrupting it', () => {
  const config = parseJsonc(`{
    "note": "x, ]",
    "a": 1
  }`);
  assert.equal(config.note, 'x, ]');
});

test('parses a string containing ", }" without corrupting it', () => {
  const config = parseJsonc(`{ "note": "x, }", "a": 1 }`);
  assert.equal(config.note, 'x, }');
});

test('still tolerates trailing commas and // line comments', () => {
  const config = parseJsonc(`{
    // a line comment
    "a": 1,
    "list": [1, 2,],
  }`);
  assert.deepEqual(config, { a: 1, list: [1, 2] });
});

test('still throws on a genuinely malformed config', () => {
  assert.throws(() => parseJsonc('{ "a": }'));
  assert.throws(() => parseJsonc('{ "a": 1'));
});

// Strict argv validation (#3297): unknown arguments are a usage error,
// reported before any fetch/discovery/write. These probes run the real
// script with an isolated HOME and an explicit OPENCODE_CONFIG_PATH so the
// temp target config — never the live config — is the only write candidate.
const scriptPath = path.join(binDir, 'sync-models.mjs');

function runWithTempConfig(args) {
  const tmpHome = fs.mkdtempSync(path.join(os.tmpdir(), 'sync-models-test-home-'));
  const configPath = path.join(tmpHome, 'opencode.jsonc');
  const before = '{\n  // temp probe config\n  "provider": {}\n}\n';
  fs.writeFileSync(configPath, before);
  const result = spawnSync(process.execPath, [scriptPath, ...args], {
    encoding: 'utf-8',
    timeout: 60000,
    env: {
      ...process.env,
      HOME: tmpHome,
      OPENCODE_CONFIG_PATH: configPath,
    },
  });
  const after = fs.readFileSync(configPath, 'utf-8');
  const backupExists = fs.existsSync(`${configPath}.backup`);
  fs.rmSync(tmpHome, { recursive: true, force: true });
  return { result, before, after, backupExists };
}

test('rejects --bogus-flag non-zero and leaves the target config byte-identical', () => {
  const { result, before, after, backupExists } = runWithTempConfig(['--bogus-flag']);
  assert.notEqual(result.status, 0);
  assert.match(`${result.stderr}${result.stdout}`, /--bogus-flag/);
  assert.equal(after, before);
  assert.equal(backupExists, false);
});

test('rejects typo --dry-rnu non-zero, names it, and leaves the target config byte-identical', () => {
  const { result, before, after, backupExists } = runWithTempConfig(['--dry-rnu']);
  assert.notEqual(result.status, 0);
  assert.match(`${result.stderr}${result.stdout}`, /--dry-rnu/);
  assert.equal(after, before);
  assert.equal(backupExists, false);
});

test('rejects a positional argument non-zero and leaves the target config byte-identical', () => {
  const { result, before, after, backupExists } = runWithTempConfig(['some-positional']);
  assert.notEqual(result.status, 0);
  assert.match(`${result.stderr}${result.stdout}`, /some-positional/);
  assert.equal(after, before);
  assert.equal(backupExists, false);
});

test('--dry-run still exits 0 writing nothing', () => {
  const { result, before, after, backupExists } = runWithTempConfig(['--dry-run']);
  assert.equal(result.status, 0);
  assert.equal(after, before);
  assert.equal(backupExists, false);
});
