/** Decode a byte stream chunk by chunk without corrupting multibyte UTF-8
 * sequences split across chunk boundaries (#4336). Per-chunk
 * `chunk.toString('utf-8')` turns a split sequence into U+FFFD on both
 * sides; StringDecoder holds the partial bytes until the rest arrives.
 * This is the streaming counterpart of request-body.mjs (#3186), which
 * covers whole-body collection. */
import { StringDecoder } from 'node:string_decoder';

export function createStreamDecoder(encoding = 'utf-8') {
  const decoder = new StringDecoder(encoding);
  return {
    push(chunk) {
      return decoder.write(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
    },
    end() {
      return decoder.end();
    },
  };
}
