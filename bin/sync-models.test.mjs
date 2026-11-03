// Tests for sync-models' parseJsonc config reader (#704).
// Run: node --test bin/sync-models.test.mjs
//
// sync-models.mjs is a top-level script (it runs the sync on import), so
// the function under test is extracted from the source and evaluated —
// the same verbatim-extraction method that reproduced #704 in triage.
// If parseJsonc's declaration shape changes, this extraction fails loudly
// rather than silently testing nothing.
import assert from 'node:assert/strict';
import fs from 'node:fs';
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
