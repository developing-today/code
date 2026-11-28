// Tests for stream-decode's chunk-boundary-safe UTF-8 decoding (#4336).
// Run: node --test bin/stream-decode.test.mjs
import assert from 'node:assert/strict';
import test from 'node:test';
import { createStreamDecoder } from './stream-decode.mjs';

const TEXT = '{"type":"assistant","text":"before-☃é😀-after"}\n';

test('preserves multibyte UTF-8 split at every byte offset (#4336)', () => {
  const bytes = Buffer.from(TEXT, 'utf-8');
  for (let split = 1; split < bytes.length; split++) {
    const decoder = createStreamDecoder();
    const out = decoder.push(bytes.subarray(0, split)) + decoder.push(bytes.subarray(split)) + decoder.end();
    assert.equal(out, TEXT, `split at byte ${split}`);
    assert.ok(!out.includes('�'), `split at byte ${split} produced U+FFFD`);
  }
});

test('preserves multibyte UTF-8 fed one byte at a time (#4336)', () => {
  const bytes = Buffer.from(TEXT, 'utf-8');
  const decoder = createStreamDecoder();
  let out = '';
  for (const byte of bytes) out += decoder.push(Buffer.from([byte]));
  out += decoder.end();
  assert.equal(out, TEXT);
});

test('JSON line survives a split inside a 3-byte sequence (#4336)', () => {
  const bytes = Buffer.from(TEXT, 'utf-8');
  const snowmanStart = Buffer.from('{"type":"assistant","text":"before-', 'utf-8').length;
  const decoder = createStreamDecoder();
  const out = decoder.push(bytes.subarray(0, snowmanStart + 1)) + decoder.push(bytes.subarray(snowmanStart + 1)) + decoder.end();
  const parsed = JSON.parse(out.trim());
  assert.equal(parsed.text, 'before-☃é😀-after');
});
