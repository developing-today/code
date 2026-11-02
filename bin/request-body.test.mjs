import assert from 'node:assert/strict';
import test from 'node:test';
import { createRequestBodyCollector } from './request-body.mjs';

test('decodes UTF-8 only after joining request body chunks', () => {
  const value = 'before-😀-after';
  const bytes = Buffer.from(JSON.stringify({ value }), 'utf8');
  const emojiStart = bytes.indexOf(Buffer.from('😀', 'utf8'));
  const collector = createRequestBodyCollector(bytes.length);

  assert.equal(collector.push(bytes.subarray(0, emojiStart + 2)), true);
  assert.equal(collector.push(bytes.subarray(emojiStart + 2)), true);
  assert.equal(collector.byteLength, bytes.length);
  assert.equal(JSON.parse(collector.text()).value, value);
});

test('enforces the request body limit in received bytes', () => {
  const collector = createRequestBodyCollector(3);

  assert.equal(collector.push(Buffer.from('a')), true);
  assert.equal(collector.push(Buffer.from('😀', 'utf8')), false);
  assert.equal(collector.tooLarge, true);
  assert.equal(collector.push(Buffer.from('b')), false);
});
